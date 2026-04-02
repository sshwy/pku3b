use bytes::Bytes;
use image::GenericImageView;
use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use std::path::Path;

/// A4 in PDF points when no image is available to infer page size.
const DEFAULT_PAGE_W: i64 = 595;
const DEFAULT_PAGE_H: i64 = 842;

pub fn images2pdf(images: &[Option<Bytes>], dest: impl AsRef<Path>) -> anyhow::Result<()> {
    let mut doc = Document::with_version("1.5");

    let pages_id = doc.new_object_id();
    let mut page_ids = Vec::new();

    let blank_dims = images
        .iter()
        .find_map(|opt| {
            opt.as_ref().and_then(|b| {
                image::load_from_memory(b.as_ref()).ok().map(|img| {
                    let (w, h) = img.dimensions();
                    (w as i64, h as i64)
                })
            })
        })
        .unwrap_or((DEFAULT_PAGE_W, DEFAULT_PAGE_H));

    for page in images {
        match page {
            Some(bytes) => {
                let img = image::load_from_memory(bytes.as_ref())?;
                let (width, height) = img.dimensions();

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
                    bytes.to_vec(),
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
            None => {
                let (w, h) = blank_dims;
                let content = Content { operations: vec![] };
                let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode()?));
                let resources_id = doc.add_object(dictionary! {});
                let page_id = doc.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), w.into(), h.into()],
                    "Contents" => content_id,
                    "Resources" => resources_id
                });
                page_ids.push(page_id);
            }
        }
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
