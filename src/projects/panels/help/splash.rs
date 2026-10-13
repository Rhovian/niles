use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use ratatui::text::{Line, Text};

const ALPHA_THRESHOLD: u8 = 128;

pub(super) struct Splash(DynamicImage);

impl Splash {
    pub(super) fn load() -> Result<Self> {
        image::load_from_memory_with_format(
            include_bytes!("niles-lockup-white.png"),
            ImageFormat::Png,
        )
        .context("decode the home splash PNG")
        .map(Self)
    }

    pub(super) fn text(&self, width: u16, rows: u16) -> Text<'static> {
        if width == 0 || rows == 0 {
            return Text::default();
        }
        // image 0.25.10 resizes with aspect ratio preserved; two square pixels per terminal
        // cell approximate its 1:2 aspect ratio. The library does not encode terminal glyphs:
        // https://docs.rs/image/0.25.10/image/enum.DynamicImage.html#method.resize
        let image = self
            .0
            .resize(u32::from(width), u32::from(rows) * 2, FilterType::Triangle)
            .into_rgba8();
        Text::from(
            (0..image.height())
                .step_by(2)
                .map(|y| {
                    let line = (0..image.width())
                        .map(|x| {
                            let top = image.get_pixel(x, y)[3] >= ALPHA_THRESHOLD;
                            let bottom = y + 1 < image.height()
                                && image.get_pixel(x, y + 1)[3] >= ALPHA_THRESHOLD;
                            match (top, bottom) {
                                (true, true) => '█',
                                (true, false) => '▀',
                                (false, true) => '▄',
                                (false, false) => ' ',
                            }
                        })
                        .collect::<String>();
                    Line::raw(line)
                })
                .collect::<Vec<_>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;

    #[test]
    fn half_blocks_preserve_transparency_and_an_odd_bottom_row() {
        let image = RgbaImage::from_fn(4, 3, |x, y| {
            let opaque = match y {
                0 => x < 2,
                1 => x == 0 || x == 2,
                _ => true,
            };
            Rgba([255, 255, 255, if opaque { 255 } else { 0 }])
        });
        let splash = Splash(DynamicImage::ImageRgba8(image));
        let text = splash.text(4, 2);
        assert_eq!(text.lines[0].to_string(), "█▀▄ ");
        assert_eq!(text.lines[1].to_string(), "▀▀▀▀");
    }

    #[test]
    fn resize_preserves_aspect_ratio_under_width_and_height_limits() {
        let splash = Splash(DynamicImage::new_rgba8(100, 50));
        let wide = splash.text(20, 20);
        assert_eq!((wide.width(), wide.height()), (20, 5));
        let short = splash.text(20, 3);
        assert_eq!((short.width(), short.height()), (12, 3));
        assert_eq!(splash.text(0, 3).height(), 0);
        assert_eq!(splash.text(20, 0).height(), 0);
    }
}
