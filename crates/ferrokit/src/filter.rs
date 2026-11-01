//! A filter builder: AppKit's `NSPredicateEditor` ([`filter_editor`])
//! editing a [`Filter`] of rules, which tests rows ([`Filter::matches`])
//! or becomes a SQL condition ([`Filter::to_sql`]).

use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadOnly, msg_send};
use objc2_app_kit::{NSPopUpButton, NSPredicateEditor, NSPredicateEditorRowTemplate, NSRuleEditorNestingMode};
use objc2_foundation::{
    NSArray, NSComparisonPredicate, NSComparisonPredicateModifier, NSComparisonPredicateOptions, NSCompoundPredicate, NSCompoundPredicateType, NSExpression, NSNumber, NSPredicate,
    NSPredicateOperatorType, NSRect, NSString,
};

use crate::signal::{Effect, Signal, untrack};
use crate::view::{Cx, Node, Size, View};

/// How a [`Filter`]'s rules combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Match {
    /// Every rule holds ("All of the following are true").
    #[default]
    All,
    /// At least one holds.
    Any,
    /// None holds.
    None,
}

/// What a rule tests a field for.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterTest {
    /// Text containing this (ignoring case).
    Contains(String),
    /// Text beginning with this (ignoring case).
    BeginsWith(String),
    /// Text ending with this (ignoring case).
    EndsWith(String),
    /// Text (or a choice) that is exactly this.
    Is(String),
    /// Text (or a choice) that isn't this.
    IsNot(String),
    /// A number equal to this.
    Equal(f64),
    /// A number other than this.
    NotEqual(f64),
    /// A number less than this.
    Less(f64),
    /// A number greater than this.
    Greater(f64),
    /// A number at most this.
    AtMost(f64),
    /// A number at least this.
    AtLeast(f64),
}

/// A rule of a [`Filter`]: a test of one field, or a group of rules.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterRule {
    /// The field with this key passes the test.
    Field {
        /// The field's key (see [`FilterField`]).
        field: String,
        /// The test.
        test: FilterTest,
    },
    /// Rules of their own, combined their own way (the user makes one by
    /// Option-clicking + in the editor).
    Group(Filter),
}

/// Rules, all, any or none of which must hold: what a [`filter_editor`]
/// edits. An empty filter lets everything through, and a number rule
/// whose number isn't filled in yet (NaN) is left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    /// How the rules combine.
    pub combine: Match,
    /// The rules.
    pub rules: Vec<FilterRule>,
}

/// A field's value, for [`Filter::matches`].
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// Text (or a choice).
    Text(String),
    /// A number.
    Number(f64),
    /// No value: every test fails on it (but "is not" holds).
    Missing,
}

impl FilterTest {
    fn holds(&self, value: &FieldValue) -> bool {
        use FilterTest::*;
        let lower = |s: &str| s.to_lowercase();
        match (self, value) {
            (Contains(t), FieldValue::Text(v)) => lower(v).contains(&lower(t)),
            (BeginsWith(t), FieldValue::Text(v)) => lower(v).starts_with(&lower(t)),
            (EndsWith(t), FieldValue::Text(v)) => lower(v).ends_with(&lower(t)),
            (Is(t), FieldValue::Text(v)) => v == t,
            (IsNot(t), FieldValue::Text(v)) => v != t,
            (IsNot(_), FieldValue::Missing) | (NotEqual(_), FieldValue::Missing) => true,
            (Equal(n), FieldValue::Number(v)) => v == n,
            (NotEqual(n), FieldValue::Number(v)) => v != n,
            (Less(n), FieldValue::Number(v)) => v < n,
            (Greater(n), FieldValue::Number(v)) => v > n,
            (AtMost(n), FieldValue::Number(v)) => v <= n,
            (AtLeast(n), FieldValue::Number(v)) => v >= n,
            _ => false,
        }
    }

    fn sql(&self, column: &str) -> String {
        use FilterTest::*;
        // A literal in single quotes (quotes doubled); in a LIKE pattern,
        // its wildcards escaped with a backslash.
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let like = |before: &str, s: &str, after: &str| {
            let escaped = s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            format!("{column} LIKE {} ESCAPE '\\'", quote(&format!("{before}{escaped}{after}")))
        };
        let num = |n: &f64| if n.is_finite() { format!("{n}") } else { "NULL".into() };
        match self {
            Contains(t) => like("%", t, "%"),
            BeginsWith(t) => like("", t, "%"),
            EndsWith(t) => like("%", t, ""),
            Is(t) => format!("{column} = {}", quote(t)),
            IsNot(t) => format!("({column} IS NULL OR {column} <> {})", quote(t)),
            Equal(n) => format!("{column} = {}", num(n)),
            NotEqual(n) => format!("({column} IS NULL OR {column} <> {})", num(n)),
            Less(n) => format!("{column} < {}", num(n)),
            Greater(n) => format!("{column} > {}", num(n)),
            AtMost(n) => format!("{column} <= {}", num(n)),
            AtLeast(n) => format!("{column} >= {}", num(n)),
        }
    }
}

impl Filter {
    /// Whether a row passes: `value(key)` gives the row's value for a
    /// field. An empty filter passes everything.
    ///
    /// ```
    /// use ferrokit::{FieldValue, Filter, FilterRule, FilterTest, Match};
    ///
    /// let big = Filter { combine: Match::All, rules: vec![FilterRule::Field { field: "total".into(), test: FilterTest::Greater(100.0) }] };
    /// assert!(big.matches(|_| FieldValue::Number(120.0)));
    /// ```
    pub fn matches(&self, value: impl Fn(&str) -> FieldValue) -> bool {
        self.matches_dyn(&value)
    }

    fn matches_dyn(&self, value: &dyn Fn(&str) -> FieldValue) -> bool {
        let holds = |r: &&FilterRule| match r {
            FilterRule::Field { field, test } => test.holds(&value(field)),
            FilterRule::Group(g) => g.matches_dyn(value),
        };
        let mut rules = self.rules.iter().filter(|r| r.complete()).peekable();
        match self.combine {
            Match::All => rules.all(|r| holds(&r)),
            Match::Any => rules.peek().is_none() || rules.any(|r| holds(&r)),
            Match::None => !rules.any(|r| holds(&r)),
        }
    }

    /// The filter as a SQL condition (for a `WHERE`): field keys become
    /// double-quoted column names and values quoted literals, so what the
    /// user types can't change the query. Text tests other than "is" ignore
    /// ASCII case, as SQLite's `LIKE` does. An empty filter is `1`.
    ///
    /// ```
    /// use ferrokit::{Filter, FilterRule, FilterTest, Match};
    ///
    /// let f = Filter { combine: Match::Any, rules: vec![
    ///     FilterRule::Field { field: "city".into(), test: FilterTest::Is("Oslo".into()) },
    ///     FilterRule::Field { field: "total".into(), test: FilterTest::AtLeast(100.0) },
    /// ] };
    /// assert_eq!(f.to_sql(), r#"("city" = 'Oslo' OR "total" >= 100)"#);
    /// ```
    pub fn to_sql(&self) -> String {
        let parts: Vec<String> = self
            .rules
            .iter()
            .filter(|r| r.complete())
            .map(|r| match r {
                FilterRule::Field { field, test } => test.sql(&format!("\"{}\"", field.replace('"', "\"\""))),
                FilterRule::Group(g) => g.to_sql(),
            })
            .collect();
        match self.combine {
            _ if parts.is_empty() => "1".into(),
            Match::All | Match::Any if parts.len() == 1 => parts[0].clone(),
            Match::All => format!("({})", parts.join(" AND ")),
            Match::Any => format!("({})", parts.join(" OR ")),
            Match::None => format!("NOT ({})", parts.join(" OR ")),
        }
    }
}

impl FilterRule {
    /// Whether the rule has its value (a number rule's number filled in).
    fn complete(&self) -> bool {
        use FilterTest::*;
        match self {
            FilterRule::Field { test: Equal(n) | NotEqual(n) | Less(n) | Greater(n) | AtMost(n) | AtLeast(n), .. } => !n.is_nan(),
            _ => true,
        }
    }
}

/// What kind of values a [`FilterField`] has, and so its tests.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    /// Text: contains, begins with, ends with, is, is not.
    Text,
    /// Numbers: is, is not, is less than, is greater than, at most, at least.
    Number,
    /// One of these choices: is, is not.
    Choice(Vec<String>),
}

/// A field a [`filter_editor`] offers: its key (in the [`Filter`]), its
/// title (in the editor) and its kind.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterField {
    /// The key rules name it by.
    pub key: String,
    /// What the editor shows.
    pub title: String,
    /// Its values.
    pub kind: FieldKind,
}

impl FilterField {
    /// A text field.
    pub fn text(key: &str, title: &str) -> Self {
        FilterField { key: key.into(), title: title.into(), kind: FieldKind::Text }
    }

    /// A number field.
    pub fn number(key: &str, title: &str) -> Self {
        FilterField { key: key.into(), title: title.into(), kind: FieldKind::Number }
    }

    /// A field with one of `choices`.
    pub fn choice(key: &str, title: &str, choices: &[&str]) -> Self {
        FilterField { key: key.into(), title: title.into(), kind: FieldKind::Choice(choices.iter().map(|c| c.to_string()).collect()) }
    }
}

// ---- to and from NSPredicate ----

const TEXT_OPS: [NSPredicateOperatorType; 5] = [
    NSPredicateOperatorType::ContainsPredicateOperatorType,
    NSPredicateOperatorType::BeginsWithPredicateOperatorType,
    NSPredicateOperatorType::EndsWithPredicateOperatorType,
    NSPredicateOperatorType::EqualToPredicateOperatorType,
    NSPredicateOperatorType::NotEqualToPredicateOperatorType,
];
const NUMBER_OPS: [NSPredicateOperatorType; 6] = [
    NSPredicateOperatorType::EqualToPredicateOperatorType,
    NSPredicateOperatorType::NotEqualToPredicateOperatorType,
    NSPredicateOperatorType::LessThanPredicateOperatorType,
    NSPredicateOperatorType::GreaterThanPredicateOperatorType,
    NSPredicateOperatorType::LessThanOrEqualToPredicateOperatorType,
    NSPredicateOperatorType::GreaterThanOrEqualToPredicateOperatorType,
];
const CHOICE_OPS: [NSPredicateOperatorType; 2] = [NSPredicateOperatorType::EqualToPredicateOperatorType, NSPredicateOperatorType::NotEqualToPredicateOperatorType];

fn op_numbers(ops: &[NSPredicateOperatorType]) -> Retained<NSArray<NSNumber>> {
    NSArray::from_retained_slice(&ops.iter().map(|o| NSNumber::new_usize(o.0)).collect::<Vec<_>>())
}

fn to_predicate(f: &Filter) -> Retained<NSPredicate> {
    let subs: Vec<Retained<NSPredicate>> = f
        .rules
        .iter()
        .map(|r| match r {
            FilterRule::Group(g) => to_predicate(g),
            FilterRule::Field { field, test } => {
                use FilterTest::*;
                let (op, text, number) = match test {
                    Contains(t) => (TEXT_OPS[0], Some(t), None),
                    BeginsWith(t) => (TEXT_OPS[1], Some(t), None),
                    EndsWith(t) => (TEXT_OPS[2], Some(t), None),
                    Is(t) => (TEXT_OPS[3], Some(t), None),
                    IsNot(t) => (TEXT_OPS[4], Some(t), None),
                    Equal(n) => (NUMBER_OPS[0], None, Some(*n)),
                    NotEqual(n) => (NUMBER_OPS[1], None, Some(*n)),
                    Less(n) => (NUMBER_OPS[2], None, Some(*n)),
                    Greater(n) => (NUMBER_OPS[3], None, Some(*n)),
                    AtMost(n) => (NUMBER_OPS[4], None, Some(*n)),
                    AtLeast(n) => (NUMBER_OPS[5], None, Some(*n)),
                };
                let constant: Retained<AnyObject> = match (text, number) {
                    (Some(t), _) => Retained::into_super(Retained::into_super(NSString::from_str(t))),
                    (_, Some(n)) => Retained::into_super(Retained::into_super(Retained::into_super(NSNumber::new_f64(n)))),
                    _ => unreachable!(),
                };
                let options = if text.is_some() && !matches!(test, Is(_) | IsNot(_)) { NSComparisonPredicateOptions::CaseInsensitivePredicateOption } else { NSComparisonPredicateOptions::empty() };
                let left = NSExpression::expressionForKeyPath(&NSString::from_str(field));
                // SAFETY: a string or number constant.
                let right = unsafe { NSExpression::expressionForConstantValue(Some(&constant)) };
                Retained::into_super(NSComparisonPredicate::predicateWithLeftExpression_rightExpression_modifier_type_options(&left, &right, NSComparisonPredicateModifier::DirectPredicateModifier, op, options))
            }
        })
        .collect();
    let array = NSArray::from_retained_slice(&subs);
    let p = match f.combine {
        Match::All => NSCompoundPredicate::andPredicateWithSubpredicates(&array),
        Match::Any => NSCompoundPredicate::orPredicateWithSubpredicates(&array),
        Match::None => NSCompoundPredicate::notPredicateWithSubpredicate(&NSCompoundPredicate::orPredicateWithSubpredicates(&array)),
    };
    Retained::into_super(p)
}

fn from_predicate(p: &NSPredicate, fields: &[FilterField]) -> Filter {
    if let Some(c) = p.downcast_ref::<NSCompoundPredicate>() {
        let subs: Vec<Retained<NSPredicate>> = c.subpredicates().iter().filter_map(|s| s.downcast::<NSPredicate>().ok()).collect();
        let kind = c.compoundPredicateType();
        // "None of": NOT around the rules (or around an OR of them).
        if kind == NSCompoundPredicateType::NotPredicateType {
            let inner = match subs.as_slice() {
                [one] => match one.downcast_ref::<NSCompoundPredicate>() {
                    Some(o) if o.compoundPredicateType() == NSCompoundPredicateType::OrPredicateType => from_predicate(one, fields).rules,
                    _ => vec![rule(one, fields)],
                },
                many => many.iter().map(|s| rule(s, fields)).collect(),
            };
            return Filter { combine: Match::None, rules: inner };
        }
        let combine = if kind == NSCompoundPredicateType::OrPredicateType { Match::Any } else { Match::All };
        return Filter { combine, rules: subs.iter().map(|s| rule(s, fields)).collect() };
    }
    Filter { combine: Match::All, rules: vec![rule(p, fields)] }
}

fn rule(p: &NSPredicate, fields: &[FilterField]) -> FilterRule {
    let Some(c) = p.downcast_ref::<NSComparisonPredicate>() else { return FilterRule::Group(from_predicate(p, fields)) };
    let field = c.leftExpression().keyPath().to_string();
    let constant = c.rightExpression().constantValue();
    let text = constant.as_ref().and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()));
    let number = constant.as_ref().and_then(|v| v.downcast_ref::<NSNumber>().map(|n| n.as_f64()));
    let numeric = fields.iter().find(|f| f.key == field).is_some_and(|f| f.kind == FieldKind::Number);
    let op = c.predicateOperatorType();
    use FilterTest::*;
    let test = if numeric {
        // An empty number field: no number yet.
        let n = number.or_else(|| text.as_deref().and_then(|t| t.trim().parse().ok())).unwrap_or(f64::NAN);
        match op {
            NSPredicateOperatorType::NotEqualToPredicateOperatorType => NotEqual(n),
            NSPredicateOperatorType::LessThanPredicateOperatorType => Less(n),
            NSPredicateOperatorType::GreaterThanPredicateOperatorType => Greater(n),
            NSPredicateOperatorType::LessThanOrEqualToPredicateOperatorType => AtMost(n),
            NSPredicateOperatorType::GreaterThanOrEqualToPredicateOperatorType => AtLeast(n),
            _ => Equal(n),
        }
    } else {
        let t = text.or_else(|| number.map(|n| n.to_string())).unwrap_or_default();
        match op {
            NSPredicateOperatorType::ContainsPredicateOperatorType => Contains(t),
            NSPredicateOperatorType::BeginsWithPredicateOperatorType => BeginsWith(t),
            NSPredicateOperatorType::EndsWithPredicateOperatorType => EndsWith(t),
            NSPredicateOperatorType::NotEqualToPredicateOperatorType => IsNot(t),
            _ => Is(t),
        }
    };
    FilterRule::Field { field, test }
}

// ---- the editor ----

/// AppKit's attribute types (Core Data's `NSAttributeType`), for a row
/// template's typed value field.
const DOUBLE_ATTRIBUTE: usize = 500;
const STRING_ATTRIBUTE: usize = 700;

fn templates(fields: &[FilterField]) -> Vec<Retained<NSPredicateEditorRowTemplate>> {
    let compound = [NSCompoundPredicateType::AndPredicateType, NSCompoundPredicateType::OrPredicateType, NSCompoundPredicateType::NotPredicateType];
    let compound = NSArray::from_retained_slice(&compound.iter().map(|c| NSNumber::new_usize(c.0)).collect::<Vec<_>>());
    let mut out = vec![NSPredicateEditorRowTemplate::initWithCompoundTypes(NSPredicateEditorRowTemplate::alloc(), &compound)];
    let keys = |kind: &dyn Fn(&FieldKind) -> bool| -> Vec<&FilterField> { fields.iter().filter(|f| kind(&f.kind)).collect() };
    let left = |fs: &[&FilterField]| NSArray::from_retained_slice(&fs.iter().map(|f| NSExpression::expressionForKeyPath(&NSString::from_str(&f.key))).collect::<Vec<_>>());
    let typed = |fs: Vec<&FilterField>, attribute: usize, ops: &[NSPredicateOperatorType], options: NSComparisonPredicateOptions| -> Option<Retained<NSPredicateEditorRowTemplate>> {
        if fs.is_empty() {
            return None;
        }
        // SAFETY: AppKit's initialiser, with an attribute type it documents.
        let t: Retained<NSPredicateEditorRowTemplate> = unsafe {
            msg_send![NSPredicateEditorRowTemplate::alloc(), initWithLeftExpressions: &*left(&fs), rightExpressionAttributeType: attribute, modifier: NSComparisonPredicateModifier::DirectPredicateModifier, operators: &*op_numbers(ops), options: options.0]
        };
        name_fields(&t, &fs);
        Some(t)
    };
    out.extend(typed(keys(&|k| *k == FieldKind::Text), STRING_ATTRIBUTE, &TEXT_OPS, NSComparisonPredicateOptions::CaseInsensitivePredicateOption));
    out.extend(typed(keys(&|k| *k == FieldKind::Number), DOUBLE_ATTRIBUTE, &NUMBER_OPS, NSComparisonPredicateOptions::empty()));
    for f in fields {
        if let FieldKind::Choice(choices) = &f.kind {
            let right: Vec<Retained<NSExpression>> = choices
                .iter()
                // SAFETY: a string constant.
                .map(|c| unsafe { NSExpression::expressionForConstantValue(Some(&NSString::from_str(c))) })
                .collect();
            let t = NSPredicateEditorRowTemplate::initWithLeftExpressions_rightExpressions_modifier_operators_options(
                NSPredicateEditorRowTemplate::alloc(),
                &left(&[f]),
                &NSArray::from_retained_slice(&right),
                NSComparisonPredicateModifier::DirectPredicateModifier,
                &op_numbers(&CHOICE_OPS),
                0,
            );
            name_fields(&t, &[f]);
            out.push(t);
        }
    }
    out
}

/// Shows fields by their titles, not their keys, in a template's first
/// pop-up.
fn name_fields(t: &NSPredicateEditorRowTemplate, fs: &[&FilterField]) {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let views = t.templateViews(mtm);
    let Some(popup) = views.firstObject().and_then(|v| v.downcast::<NSPopUpButton>().ok()) else { return };
    for (i, f) in fs.iter().enumerate() {
        if let Some(item) = popup.itemAtIndex(i as isize) {
            item.setTitle(&NSString::from_str(&f.title));
        }
    }
}

/// Names each row's controls for VoiceOver (AppKit leaves them unnamed):
/// the line's pop-up "Match", a rule's "Field", "Test" and "Value".
fn name_rows(editor: &NSPredicateEditor) {
    use objc2_app_kit::{NSControl, NSView};
    fn controls(v: &NSView, out: &mut Vec<Retained<NSControl>>) {
        for s in v.subviews().iter() {
            match s.downcast::<NSControl>() {
                Ok(c) if c.downcast_ref::<objc2_app_kit::NSButton>().is_none() || c.downcast_ref::<NSPopUpButton>().is_some() => out.push(c),
                Ok(_) => {}
                Err(s) => controls(&s, out),
            }
        }
    }
    let mut all = Vec::new();
    controls(editor, &mut all);
    // By row (their superview), left to right.
    let mut rows: Vec<(usize, Vec<Retained<NSControl>>)> = Vec::new();
    for c in all {
        let Some(parent) = (unsafe { c.superview() }) else { continue };
        let key = Retained::as_ptr(&parent) as usize;
        match rows.iter_mut().find(|r| r.0 == key) {
            Some(r) => r.1.push(c),
            None => rows.push((key, vec![c])),
        }
    }
    for (_, mut cs) in rows {
        cs.sort_by(|a, b| a.frame().origin.x.total_cmp(&b.frame().origin.x));
        let names: &[&str] = if cs.len() == 1 { &["Match"] } else { &["Field", "Test", "Value"] };
        for (c, name) in cs.iter().zip(names) {
            <crate::platform::Native as crate::platform::Platform>::set_accessibility_label(c, &crate::t(name));
        }
    }
}

/// Equal filters, a number not filled in (NaN) equal to another: so the
/// editor isn't reset while the user fills one in.
fn same(a: &Filter, b: &Filter) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

/// A filter builder (see [`filter_editor`]).
pub struct FilterEditor {
    filter: Signal<Filter>,
    fields: Rc<Vec<FilterField>>,
}

impl View for FilterEditor {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        // SAFETY: NSView's designated initialiser.
        let editor: Retained<NSPredicateEditor> = unsafe { msg_send![NSPredicateEditor::alloc(mtm), initWithFrame: NSRect::ZERO] };
        let rows = NSArray::from_retained_slice(&templates(&self.fields));
        editor.setRowTemplates(&rows);
        editor.setNestingMode(NSRuleEditorNestingMode::Compound);
        editor.setCanRemoveAllRows(false);
        let view = Retained::into_super(Retained::into_super(Retained::into_super(editor.clone())));
        let (mut node, leaf) = Node::leaf(cx, view, (true, false), Size { w: 360.0, h: 0.0 });
        let leaf = Rc::new(leaf);
        // As tall as its rows.
        let fit = {
            let (e, leaf) = (editor.clone(), leaf.clone());
            move || {
                name_rows(&e);
                let h = e.numberOfRows() as f64 * e.rowHeight();
                let new = Size { w: 360.0, h };
                if leaf.content.get() != Some(new) {
                    leaf.content.set(Some(new));
                    leaf.meta.mark();
                    crate::later(crate::relayout_now);
                }
            }
        };
        let fit = Rc::new(fit);
        // The user's changes set the filter.
        let (filter, fields, e, f2) = (self.filter, self.fields.clone(), editor.clone(), fit.clone());
        let target = crate::appkit_views::ActionTarget::new(mtm, move || {
            if let Some(p) = e.predicate() {
                let new = from_predicate(&p, &fields);
                if !same(&filter.get_untracked(), &new) {
                    filter.set(new);
                }
            }
            f2();
        });
        // SAFETY: the target responds to `fire:`; the node keeps it alive.
        unsafe {
            editor.setTarget(Some(&target));
            editor.setAction(Some(objc2::sel!(fire:)));
        }
        node.keep(Retained::into_super(target));
        // And the filter, set from code, shows.
        let (filter, fields, e) = (self.filter, self.fields, editor.clone());
        node.own(Effect::new(move || {
            let want = filter.get();
            untrack(|| {
                let shown = e.predicate().map(|p| from_predicate(&p, &fields));
                if !shown.as_ref().is_some_and(|s| same(s, &want)) {
                    let mut want = want.clone();
                    // The editor needs a row under its "All of" line.
                    if want.rules.is_empty()
                        && let Some(first) = fields.first()
                    {
                        let test = match first.kind {
                            FieldKind::Number => FilterTest::Equal(0.0),
                            FieldKind::Text => FilterTest::Contains(String::new()),
                            FieldKind::Choice(ref c) => FilterTest::Is(c.first().cloned().unwrap_or_default()),
                        };
                        want.rules.push(FilterRule::Field { field: first.key.clone(), test });
                    }
                    // SAFETY: an NSPredicate is the editor's object value.
                    unsafe { e.setObjectValue(Some(&to_predicate(&want))) };
                }
                fit();
            });
        }));
        node
    }
}

/// AppKit's filter builder (`NSPredicateEditor`, as Mail's rules and
/// Finder's smart folders use), bound two ways to `filter`: a line saying
/// whether all, any or none of the rules must hold, then a row per rule
/// (a field, a test, a value), with + and − to add and remove rows; Option
/// with + adds a nested group. It grows as rows are added. Text fields
/// test contains, begins with, ends with, is and is not; numbers compare;
/// choices are, or aren't, one of theirs.
///
/// ```no_run
/// use ferrokit::*;
///
/// let filter = Signal::new(Filter::default());
/// let editor = filter_editor(&filter, vec![
///     FilterField::text("name", "Name"),
///     FilterField::number("total", "Total"),
///     FilterField::choice("city", "City", &["Oslo", "Bergen", "Tromsø"]),
/// ]);
/// let condition = move || filter.get().to_sql(); // for a WHERE
/// # let _ = (editor, condition);
/// ```
pub fn filter_editor(filter: &Signal<Filter>, fields: Vec<FilterField>) -> FilterEditor {
    FilterEditor { filter: *filter, fields: Rc::new(fields) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(f: &str, test: FilterTest) -> FilterRule {
        FilterRule::Field { field: f.into(), test }
    }

    #[test]
    fn sql_quotes_what_users_type() {
        let f = Filter { combine: Match::All, rules: vec![field("name", FilterTest::Contains("O'Brien 50%_".into())), field("we\"ird", FilterTest::Is("x".into()))] };
        assert_eq!(f.to_sql(), r#"("name" LIKE '%O''Brien 50\%\_%' ESCAPE '\' AND "we""ird" = 'x')"#);
        let none = Filter { combine: Match::None, rules: vec![field("total", FilterTest::Less(5.0)), FilterRule::Group(Filter { combine: Match::Any, rules: vec![field("city", FilterTest::IsNot("Oslo".into()))] })] };
        assert_eq!(none.to_sql(), r#"NOT ("total" < 5 OR ("city" IS NULL OR "city" <> 'Oslo'))"#);
        assert_eq!(Filter::default().to_sql(), "1");
        // A number not filled in yet: left out.
        let pending = Filter { combine: Match::Any, rules: vec![field("total", FilterTest::Equal(f64::NAN)), field("city", FilterTest::Is("Oslo".into()))] };
        assert_eq!(pending.to_sql(), r#""city" = 'Oslo'"#);
        assert!(!pending.matches(|_| FieldValue::Text("Bergen".into())));
    }

    #[test]
    fn rows_match() {
        let f = Filter {
            combine: Match::Any,
            rules: vec![field("name", FilterTest::BeginsWith("ad".into())), FilterRule::Group(Filter { combine: Match::All, rules: vec![field("total", FilterTest::AtLeast(100.0)), field("city", FilterTest::Is("Oslo".into()))] })],
        };
        let row = |name: &str, total: f64, city: &str| {
            let (name, city) = (name.to_string(), city.to_string());
            move |k: &str| match k {
                "name" => FieldValue::Text(name.clone()),
                "total" => FieldValue::Number(total),
                "city" => FieldValue::Text(city.clone()),
                _ => FieldValue::Missing,
            }
        };
        assert!(f.matches(row("Ada", 5.0, "Bergen")));
        assert!(f.matches(row("Bo", 150.0, "Oslo")));
        assert!(!f.matches(row("Bo", 150.0, "Bergen")));
        assert!(Filter::default().matches(|_| FieldValue::Missing));
        assert!(!Filter { combine: Match::None, rules: vec![field("name", FilterTest::Contains("o".into()))] }.matches(row("Bo", 1.0, "")));
    }
}
