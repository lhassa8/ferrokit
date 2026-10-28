//! Pictures of native views, for drawing in the host while the real view is
//! hidden under a host overlay (risk 1, "snapshot in place").

use crate::ViewId;
use crate::host::with_host;

/// RGBA8 pixels, unpremultiplied as AppKit returns them, at backing resolution.
#[derive(Clone)]
pub struct Snapshot {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels, row by row from the top.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Snapshot({}x{})", self.width, self.height)
    }
}

/// Renders the mounted view into a bitmap, as it currently looks.
pub fn snapshot(id: ViewId) -> Option<Snapshot> {
    with_host(|host| {
        let view = &host.views.get(&id)?.view;
        let bounds = view.bounds();
        let rep = view.bitmapImageRepForCachingDisplayInRect(bounds)?;
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);

        let (w, h) = (rep.pixelsWide() as usize, rep.pixelsHigh() as usize);
        let (bpr, spp) = (rep.bytesPerRow() as usize, rep.samplesPerPixel() as usize);
        let data = rep.bitmapData();
        if data.is_null() || rep.bitsPerSample() != 8 || !(3..=4).contains(&spp) {
            return None;
        }
        let alpha_first = rep.bitmapFormat().contains(objc2_app_kit::NSBitmapFormat::AlphaFirst);
        // SAFETY: bitmapData points at bytesPerRow * pixelsHigh bytes owned by `rep`.
        let src = unsafe { std::slice::from_raw_parts(data, bpr * h) };
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in src.chunks(bpr).take(h) {
            for px in row[..w * spp].chunks(spp) {
                let (r, g, b, a) = match (spp, alpha_first) {
                    (4, true) => (px[1], px[2], px[3], px[0]),
                    (4, false) => (px[0], px[1], px[2], px[3]),
                    _ => (px[0], px[1], px[2], 255),
                };
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
        Some(Snapshot { width: w as u32, height: h as u32, rgba })
    })
    .flatten()
}
