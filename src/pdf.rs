use image::GenericImageView;
use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn images2pdf(images: &[PathBuf], dest: impl AsRef<Path>) -> anyhow::Result<()> {
    let mut doc = Document::with_version("1.5");

    let pages_id = doc.new_object_id();
    let mut page_ids = Vec::new();

    for img_path in images {
        let img = image::open(img_path)?;
        let (width, height) = img.dimensions();

        let img_data = fs::read(img_path)?;

        let image_id = doc.new_object_id();

        let image_stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => width as i64,
                "Height" => height as i64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode" // JPEG
            },
            img_data,
        );

        doc.objects.insert(image_id, Object::Stream(image_stream));

        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        width.into(),
                        0.into(),
                        0.into(),
                        height.into(),
                        0.into(),
                        0.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };

        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode()?));

        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! {
                "Im0" => image_id
            }
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(),0.into(),width.into(),height.into()],
            "Contents" => content_id,
            "Resources" => resources_id
        });

        page_ids.push(page_id);
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids
                .iter()
                .copied()
                .map(Object::Reference)
                .collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64
        }),
    );

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id
    });

    doc.trailer.set("Root", catalog_id);

    doc.compress();
    doc.save(dest)?;

    Ok(())
}
