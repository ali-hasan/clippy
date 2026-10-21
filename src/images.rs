//! Copied images, app icons and SF Symbols: converting them to PNG and drawing them into plain
//! RGBA pixels.

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSFontWeightRegular,
    NSGraphicsContext, NSImage, NSImageSymbolConfiguration,
};
use objc2_foundation::{NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use crate::history::{NewImage, Thumbnail};

/// Longest side of an image's thumbnail, in pixels; Retina screens show it at half that in points.
const THUMBNAIL_PIXELS: u32 = 256;

/// Turns copied image data, in any format macOS can read, into what the history keeps: the image
/// as PNG, its size in pixels, and a thumbnail.
pub fn prepare_image(data: &NSData, is_png: bool) -> Option<NewImage> {
    let bitmap = NSBitmapImageRep::imageRepWithData(data)?;
    let (width, height) = (bitmap.pixelsWide().max(1) as u32, bitmap.pixelsHigh().max(1) as u32);
    let png = if is_png {
        data.to_vec()
    } else {
        let no_options = NSDictionary::new();
        unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &no_options) }?.to_vec()
    };
    let image = NSImage::initWithData(NSImage::alloc(), data)?;
    let (thumbnail_width, thumbnail_height) = fit_within(width, height, THUMBNAIL_PIXELS);
    let pixels = draw_into_pixels(&image, thumbnail_width as usize, thumbnail_height as usize)?;
    let thumbnail = Thumbnail { width: thumbnail_width, height: thumbnail_height, pixels };
    Some(NewImage { png, width, height, thumbnail })
}

/// Shrinks a size to fit a square with sides of `longest`, keeping its shape. Smaller sizes are
/// left as they are.
fn fit_within(width: u32, height: u32, longest: u32) -> (u32, u32) {
    let scale = (longest as f64 / width.max(height) as f64).min(1.0);
    let scaled = |side: u32| ((side as f64 * scale).round() as u32).max(1);
    (scaled(width), scaled(height))
}

/// An icon from SF Symbols, the set macOS uses in its own menus and menu bar.
pub fn symbol(name: &str, point_size: f64) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(point_size, unsafe { NSFontWeightRegular });
    image.imageWithSymbolConfiguration(&configuration)
}

/// Draws `image` stretched over `width` × `height` pixels and returns their RGBA bytes, with each
/// color already multiplied by its alpha.
pub fn draw_into_pixels(image: &NSImage, width: usize, height: usize) -> Option<Vec<u8>> {
    // With no buffer passed in, the bitmap allocates and owns its own pixels.
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width as isize,
            height as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            width as isize * 4,
            32,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let area = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width as f64, height as f64));
    image.drawInRect_fromRect_operation_fraction(area, NSRect::ZERO, NSCompositingOperation::Copy, 1.0);
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();

    let pixels = unsafe { std::slice::from_raw_parts(bitmap.bitmapData(), width * height * 4) };
    Some(pixels.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_keep_the_shape_of_the_image() {
        assert_eq!(fit_within(1920, 1080, 256), (256, 144));
        assert_eq!(fit_within(1080, 1920, 256), (144, 256));
    }

    #[test]
    fn small_images_are_not_enlarged_for_their_thumbnail() {
        assert_eq!(fit_within(40, 30, 256), (40, 30));
    }

    #[test]
    fn very_thin_images_still_get_a_thumbnail_at_least_one_pixel_wide() {
        assert_eq!(fit_within(10_000, 2, 256), (256, 1));
    }
}
