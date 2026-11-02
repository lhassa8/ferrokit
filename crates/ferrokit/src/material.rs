//! Materials: the system's translucent backgrounds (`NSVisualEffectView`),
//! which blur what's behind them and let text over them take on vibrancy,
//! and Liquid Glass (`NSGlassEffectView`, macOS 26) with a material in its
//! place on earlier systems. Like surfaces, they sit behind a view's own
//! native views, framed by layout to its rectangle.

use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSGlassEffectView, NSGlassEffectViewStyle, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
};
use objc2_foundation::NSPoint;

use crate::Color;

/// A system material behind a view (see
/// [`ViewExt::material`](crate::ViewExt::material)): translucent, blurring
/// what's behind it, and following Dark Mode, Reduce Transparency and the
/// window's active state as the system's own sidebars and panels do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Material {
    /// A source list or sidebar, showing the desktop through the window.
    Sidebar,
    /// A section or table header over scrolling content.
    Header,
    /// A window's title bar or toolbar area.
    Titlebar,
    /// A menu.
    Menu,
    /// A popover.
    Popover,
    /// A heads-up display: a dark floating panel.
    Hud,
    /// A sheet.
    Sheet,
    /// A window's background.
    Window,
    /// The background of content (a document, a list).
    Content,
    /// Behind a window's content, showing the desktop.
    UnderWindow,
    /// Behind pages (as in Preview's page view).
    UnderPage,
    /// A selection highlight (follows the accent colour).
    Selection,
    /// A tooltip.
    ToolTip,
}

impl Material {
    fn ns(self) -> NSVisualEffectMaterial {
        match self {
            Material::Sidebar => NSVisualEffectMaterial::Sidebar,
            Material::Header => NSVisualEffectMaterial::HeaderView,
            Material::Titlebar => NSVisualEffectMaterial::Titlebar,
            Material::Menu => NSVisualEffectMaterial::Menu,
            Material::Popover => NSVisualEffectMaterial::Popover,
            Material::Hud => NSVisualEffectMaterial::HUDWindow,
            Material::Sheet => NSVisualEffectMaterial::Sheet,
            Material::Window => NSVisualEffectMaterial::WindowBackground,
            Material::Content => NSVisualEffectMaterial::ContentBackground,
            Material::UnderWindow => NSVisualEffectMaterial::UnderWindowBackground,
            Material::UnderPage => NSVisualEffectMaterial::UnderPageBackground,
            Material::Selection => NSVisualEffectMaterial::Selection,
            Material::ToolTip => NSVisualEffectMaterial::ToolTip,
        }
    }

    /// Whether it blends with what's behind the window (the desktop, other
    /// windows), as the system uses it, or with the window's own content.
    fn behind_window(self) -> bool {
        !matches!(self, Material::Header | Material::Titlebar | Material::Content | Material::UnderPage | Material::Selection)
    }
}

/// Liquid Glass behind a view (see [`ViewExt::glass`](crate::ViewExt::glass)).
/// On macOS 15, where there's no Liquid Glass, a translucent popover
/// material takes its place.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Glass {
    /// The standard glass: frosted, for controls and panels.
    Regular,
    /// Clearer glass, for content over rich backgrounds (photos, video).
    Clear,
    /// Regular glass tinted with a colour (give it some transparency).
    Tinted(Color),
}

/// Whether this Mac has Liquid Glass (macOS 26 and later).
pub fn has_liquid_glass() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// What goes behind a view's surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Backdrop {
    Material(Material),
    Glass(Glass),
}

define_class!(
    // A material view that passes clicks through (and lets a window with
    // full-size content be dragged by it).
    #[unsafe(super(NSVisualEffectView, NSView, objc2_app_kit::NSResponder, objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitMaterial"]
    pub(crate) struct MaterialView;

    impl MaterialView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

fn material_view(mtm: MainThreadMarker, material: NSVisualEffectMaterial, behind: bool, radius: f64) -> Retained<NSView> {
    // SAFETY: NSView's designated initialiser.
    let view: Retained<MaterialView> = unsafe { msg_send![MaterialView::alloc(mtm), init] };
    view.setMaterial(material);
    view.setBlendingMode(if behind { NSVisualEffectBlendingMode::BehindWindow } else { NSVisualEffectBlendingMode::WithinWindow });
    view.setState(NSVisualEffectState::FollowsWindowActiveState);
    if radius > 0.0 {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(radius);
            layer.setMasksToBounds(true);
        }
    }
    Retained::into_super(Retained::into_super(view))
}

/// The view that draws `backdrop`, with corners rounded to `radius`.
pub(crate) fn backdrop(mtm: MainThreadMarker, backdrop: Backdrop, radius: f64) -> Retained<NSView> {
    match backdrop {
        Backdrop::Material(m) => material_view(mtm, m.ns(), m.behind_window(), radius),
        Backdrop::Glass(g) if has_liquid_glass() => {
            let view = NSGlassEffectView::new(mtm);
            view.setCornerRadius(radius);
            match g {
                Glass::Regular => {}
                Glass::Clear => view.setStyle(NSGlassEffectViewStyle::Clear),
                Glass::Tinted(c) => view.setTintColor(Some(&c.ns())),
            }
            Retained::into_super(view)
        }
        // Before Liquid Glass: a popover's material, the nearest look.
        Backdrop::Glass(_) => material_view(mtm, NSVisualEffectMaterial::Popover, false, radius),
    }
}
