//! Sanity check for printpdf + CJK font + raster image embedding.
use printpdf::*;
use std::fs::File;
use std::io::BufWriter;

fn main() -> anyhow::Result<()> {
    let font_bytes: &[u8] = include_bytes!("../../assets/NotoSansSC.ttf");

    let (doc, page1, layer1) = PdfDocument::new("pdfcheck", Mm(210.0), Mm(297.0), "Layer 1");
    let layer = doc.get_page(page1).get_layer(layer1);
    let font = doc.add_external_font(font_bytes)?;
    layer.use_text("考生座次表 身份证号 姓名", 16.0, Mm(20.0), Mm(270.0), &font);
    layer.use_text(
        "加权总成绩 = 笔试成绩*0.6 + 面试成绩*0.4",
        10.0,
        Mm(20.0),
        Mm(255.0),
        &font,
    );

    // raster image from a DynamicImage
    let img =
        printpdf::image_crate::DynamicImage::ImageRgb8(printpdf::image_crate::RgbImage::from_fn(
            80,
            100,
            |x, y| printpdf::image_crate::Rgb([(x * 3) as u8, (y * 2) as u8, 120]),
        ));
    let xobj = ImageXObject::from_dynamic_image(&img);
    Image::from(xobj).add_to_layer(
        layer.clone(),
        ImageTransform {
            translate_x: Some(Mm(120.0)),
            translate_y: Some(Mm(200.0)),
            rotate: None,
            scale_x: Some(1.0),
            scale_y: Some(1.0),
            dpi: Some(96.0),
        },
    );

    // stroked rectangle + line
    layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    layer.set_outline_thickness(0.5);
    layer.add_rect(Rect::new(Mm(20.0), Mm(20.0), Mm(80.0), Mm(60.0)));
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(20.0), Mm(70.0)), false),
            (Point::new(Mm(80.0), Mm(70.0)), false),
        ],
        is_closed: false,
    });

    doc.save(&mut BufWriter::new(File::create("/tmp/out.pdf")?))?;
    println!("pdf ok");
    Ok(())
}
