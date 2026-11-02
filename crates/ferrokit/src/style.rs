//! Fonts: the system's text styles (title, headline, body, caption…) and
//! custom ones, for [`text`](crate::text), [`label`](crate::label) and other
//! views that show text.

use objc2::rc::Retained;
use objc2_app_kit::{NSFont, NSFontDescriptorSymbolicTraits};
use objc2_foundation::{NSDictionary, NSString};

/// How heavy a font's strokes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// The thinnest.
    UltraLight,
    /// Thin.
    Thin,
    /// Light.
    Light,
    /// The usual weight.
    Regular,
    /// Slightly heavier than regular.
    Medium,
    /// Between medium and bold: titles, emphasis.
    Semibold,
    /// Bold.
    Bold,
    /// Heavier than bold.
    Heavy,
    /// The heaviest.
    Black,
}

impl Weight {
    pub(crate) fn ns(self) -> f64 {
        // SAFETY: AppKit's weight constants, read on use.
        unsafe {
            use objc2_app_kit::*;
            match self {
                Weight::UltraLight => NSFontWeightUltraLight,
                Weight::Thin => NSFontWeightThin,
                Weight::Light => NSFontWeightLight,
                Weight::Regular => NSFontWeightRegular,
                Weight::Medium => NSFontWeightMedium,
                Weight::Semibold => NSFontWeightSemibold,
                Weight::Bold => NSFontWeightBold,
                Weight::Heavy => NSFontWeightHeavy,
                Weight::Black => NSFontWeightBlack,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Base {
    /// One of the system's text styles.
    Style(Style),
    /// The system font at a size.
    System(f64),
    /// The system's monospaced font at a size.
    Monospaced(f64),
    /// A font by family or PostScript name (the system font if it isn't
    /// installed).
    Named(String, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    LargeTitle,
    Title,
    Title2,
    Title3,
    Headline,
    Subheadline,
    Body,
    Callout,
    Footnote,
    Caption,
    Caption2,
}

/// A font: one of the system's text styles (sized as the Mac sizes them:
/// `body` is 13 pt, `large_title` 26 pt), or a custom one. Change its
/// weight, size or slant with the methods.
///
/// ```no_run
/// use ferrokit::*;
///
/// let header = vstack((
///     text("Revenue").font(Font::headline()).color(Color::Secondary),
///     text("$48,210").font(Font::large_title().weight(Weight::Bold).monospaced_digits()),
/// ));
/// # let _ = header;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    base: Base,
    weight: Option<Weight>,
    size: Option<f64>,
    italic: bool,
    digits: bool,
}

macro_rules! styles {
    ($($name:ident => $style:ident, $doc:literal;)*) => {
        $(
            #[doc = $doc]
            pub fn $name() -> Font {
                Font::new(Base::Style(Style::$style))
            }
        )*
    };
}

impl Font {
    fn new(base: Base) -> Font {
        Font { base, weight: None, size: None, italic: false, digits: false }
    }

    styles! {
        large_title => LargeTitle, "The largest title (26 pt).";
        title => Title, "A window or section title (22 pt).";
        title2 => Title2, "A smaller title (17 pt).";
        title3 => Title3, "A smaller title still (15 pt).";
        headline => Headline, "Emphasised body text, as in a list's headings (13 pt, bold).";
        subheadline => Subheadline, "Secondary headings (11 pt).";
        body => Body, "Body text, what labels use (13 pt).";
        callout => Callout, "Text set apart from the body (12 pt).";
        footnote => Footnote, "Footnotes (10 pt).";
        caption => Caption, "Captions (10 pt).";
        caption2 => Caption2, "Secondary captions (10 pt).";
    }

    /// The system font at `size` points.
    pub fn system(size: f64) -> Font {
        Font::new(Base::System(size))
    }

    /// The system's monospaced font (SF Mono) at `size` points.
    pub fn monospaced(size: f64) -> Font {
        Font::new(Base::Monospaced(size))
    }

    /// A font by family or PostScript name (`"Avenir Next"`,
    /// `"Georgia-Bold"`) at `size` points; the system font if it isn't
    /// installed.
    pub fn named(name: &str, size: f64) -> Font {
        Font::new(Base::Named(name.into(), size))
    }

    /// The same font at another weight.
    pub fn weight(mut self, weight: Weight) -> Font {
        self.weight = Some(weight);
        self
    }

    /// The same font, bold.
    pub fn bold(self) -> Font {
        self.weight(Weight::Bold)
    }

    /// The same font at `points` (a text style keeps its weight).
    pub fn size(mut self, points: f64) -> Font {
        self.size = Some(points);
        self
    }

    /// The same font, italic (slanted, for fonts without an italic face).
    pub fn italic(mut self) -> Font {
        self.italic = true;
        self
    }

    /// Digits all the same width, so changing numbers don't shift (totals,
    /// timers, tables of figures). The system font only.
    pub fn monospaced_digits(mut self) -> Font {
        self.digits = true;
        self
    }

    /// The `NSFont`.
    pub fn ns(&self) -> Retained<NSFont> {
        let (size, weight) = match &self.base {
            Base::Style(style) => {
                let f = preferred(*style);
                (f.pointSize(), if *style == Style::Headline { Weight::Bold } else { Weight::Regular })
            }
            Base::System(s) | Base::Monospaced(s) | Base::Named(_, s) => (*s, Weight::Regular),
        };
        let size = self.size.unwrap_or(size);
        let weight = self.weight.unwrap_or(weight);
        let font = match &self.base {
            Base::Monospaced(_) => NSFont::monospacedSystemFontOfSize_weight(size, weight.ns()),
            Base::Named(name, _) => NSFont::fontWithName_size(&NSString::from_str(name), size)
                .map(|f| if self.weight.is_some() { with_weight(&f, weight) } else { f })
                .unwrap_or_else(|| NSFont::systemFontOfSize_weight(size, weight.ns())),
            _ if self.digits => NSFont::monospacedDigitSystemFontOfSize_weight(size, weight.ns()),
            Base::Style(style) if self.weight.is_none() && self.size.is_none() => preferred(*style),
            _ => NSFont::systemFontOfSize_weight(size, weight.ns()),
        };
        if self.italic { italic(&font) } else { font }
    }
}

impl Default for Font {
    /// The body font.
    fn default() -> Font {
        Font::body()
    }
}

fn preferred(style: Style) -> Retained<NSFont> {
    // SAFETY: AppKit's text style constants, and an empty options dictionary.
    unsafe {
        use objc2_app_kit::*;
        let name: &objc2_app_kit::NSFontTextStyle = match style {
            Style::LargeTitle => NSFontTextStyleLargeTitle,
            Style::Title => NSFontTextStyleTitle1,
            Style::Title2 => NSFontTextStyleTitle2,
            Style::Title3 => NSFontTextStyleTitle3,
            Style::Headline => NSFontTextStyleHeadline,
            Style::Subheadline => NSFontTextStyleSubheadline,
            Style::Body => NSFontTextStyleBody,
            Style::Callout => NSFontTextStyleCallout,
            Style::Footnote => NSFontTextStyleFootnote,
            Style::Caption => NSFontTextStyleCaption1,
            Style::Caption2 => NSFontTextStyleCaption2,
        };
        NSFont::preferredFontForTextStyle_options(name, &NSDictionary::new())
    }
}

/// `font` at `weight` through its descriptor (for named fonts).
fn with_weight(font: &NSFont, weight: Weight) -> Retained<NSFont> {
    let bold = matches!(weight, Weight::Semibold | Weight::Bold | Weight::Heavy | Weight::Black);
    if !bold {
        return font.retain();
    }
    let d = font.fontDescriptor().fontDescriptorWithSymbolicTraits(font.fontDescriptor().symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitBold);
    NSFont::fontWithDescriptor_size(&d, font.pointSize()).unwrap_or_else(|| font.retain())
}

fn italic(font: &NSFont) -> Retained<NSFont> {
    let d = font.fontDescriptor().fontDescriptorWithSymbolicTraits(font.fontDescriptor().symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic);
    NSFont::fontWithDescriptor_size(&d, font.pointSize()).unwrap_or_else(|| font.retain())
}

use objc2::Message;
