//! Window screenshots as PNG files, for `--screenshot` and the control channel's `ui.screenshot`. A few frames are
//! drawn first so the layout has settled. Adapted from pixel3d.

use std::path::{Path, PathBuf};

/// Frames drawn before the screenshot is taken.
const SETTLE_FRAMES: u32 = 4;

/// Screenshots being taken.
#[derive(Debug, Default)]
pub struct Shots {
    pending: Option<(PathBuf, u32, u64)>,
    issued: Option<(PathBuf, u64)>,
    next: u64,
    /// The last finished screenshot: its path, or what went wrong.
    pub done: Option<Result<PathBuf, String>>,
}

impl Shots {
    /// Asks for a screenshot saved to `path`.
    pub fn request(&mut self, path: PathBuf) {
        self.next += 1;
        self.pending = Some((path, 0, self.next));
        self.done = None;
    }

    /// Whether a screenshot is under way.
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.issued.is_some()
    }

    /// Asks egui for the screenshot once the frames have settled, and saves one that arrived. Call once per frame.
    ///
    /// Complexity: O(pixels) when a screenshot arrives, O(1) otherwise.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if let Some((path, frames, token)) = self.pending.take() {
            if frames >= SETTLE_FRAMES {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(token)));
                self.issued = Some((path, token));
            } else {
                self.pending = Some((path, frames + 1, token));
            }
            ctx.request_repaint();
        }
        let Some((_, token)) = &self.issued else { return };
        let token = *token;
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. } if user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()) == Some(&token) => {
                    Some(image.clone())
                }
                _ => None,
            })
        });
        ctx.request_repaint();
        if let Some(image) = image
            && let Some((path, _)) = self.issued.take()
        {
            self.done = Some(save_png(&image, &path).map(|()| path));
        }
    }
}

/// Writes `image` to `path` as a PNG.
///
/// Complexity: O(pixels).
pub fn save_png(image: &egui::ColorImage, path: &Path) -> Result<(), String> {
    let [w, h] = image.size;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    let buffer = image::RgbaImage::from_raw(w as u32, h as u32, bytes).ok_or_else(|| String::from("the screenshot has the wrong size"))?;
    buffer.save_with_format(path, image::ImageFormat::Png).map_err(|e| format!("cannot write {}: {e}", path.display()))
}
