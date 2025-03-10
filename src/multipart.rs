use rand::{Rng, distr::Alphanumeric};
use std::io::Read;

/// 结构化的表单字段
pub struct FormField<'a> {
    name: &'a str,
    filename: Option<&'a str>,
    content_type: Option<&'a str>,
    reader: Option<Box<dyn Read + Send + 'static>>,
    data: Option<&'a [u8]>,
}

/// Multipart 表单构造器
pub struct MultipartBuilder<'a> {
    boundary: String,
    fields: Vec<FormField<'a>>,
}

impl<'a> MultipartBuilder<'a> {
    /// 创建一个新的 MultipartBuilder
    pub fn new() -> Self {
        let boundary: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();

        Self {
            boundary: format!("----WebKitFormBoundary{}", boundary),
            fields: Vec::new(),
        }
    }

    /// 添加一个普通字段
    pub fn add_field(mut self, name: &'a str, data: &'a [u8]) -> Self {
        self.fields.push(FormField {
            name,
            filename: None,
            content_type: None,
            reader: None,
            data: Some(data),
        });
        self
    }

    /// 添加一个带文件名的字段
    pub fn add_file<R: Read + Send + 'static>(
        mut self,
        name: &'a str,
        filename: &'a str,
        content_type: &'a str,
        reader: R,
    ) -> Self {
        self.fields.push(FormField {
            name,
            filename: Some(filename),
            content_type: Some(content_type),
            reader: Some(Box::new(reader)),
            data: None,
        });
        self
    }

    /// 构建 multipart/form-data body
    pub fn build(mut self) -> anyhow::Result<Vec<u8>> {
        let mut output = Vec::new();
        let dash_boundary = format!("--{}", self.boundary);

        for field in &mut self.fields {
            output.extend_from_slice(dash_boundary.as_bytes());
            output.extend_from_slice(b"\r\n");

            // Content-Disposition
            output.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{}\"", field.name).as_bytes(),
            );
            if let Some(filename) = field.filename {
                output.extend_from_slice(format!("; filename=\"{}\"", filename).as_bytes());
            }
            output.extend_from_slice(b"\r\n");

            // Content-Type (optional)
            if let Some(content_type) = field.content_type {
                output.extend_from_slice(format!("Content-Type: {}\r\n", content_type).as_bytes());
            }
            output.extend_from_slice(b"\r\n");

            // 读取数据
            if let Some(data) = field.data {
                output.extend_from_slice(data);
            } else if let Some(reader) = field.reader.as_mut() {
                std::io::copy(reader, &mut output)?;
            }
            output.extend_from_slice(b"\r\n");
        }

        // 结束 boundary
        output.extend_from_slice(dash_boundary.as_bytes());
        output.extend_from_slice(b"--\r\n");

        Ok(output)
    }

    /// 获取 boundary 字符串
    pub fn boundary(&self) -> &str {
        &self.boundary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_multipart_builder() {
        let file_content = b"File content";
        let file_reader = Cursor::new(file_content);

        let builder = MultipartBuilder::new()
            .add_field("field1", b"Hello, world!")
            .add_file("field2", "file.txt", "text/plain", file_reader);

        let body = builder.build().unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        assert!(body_str.contains("Content-Disposition: form-data; name=\"field1\""));
        assert!(body_str.contains("Hello, world!"));
        assert!(
            body_str
                .contains("Content-Disposition: form-data; name=\"field2\"; filename=\"file.txt\"")
        );
        assert!(body_str.contains("Content-Type: text/plain"));
        assert!(body_str.contains("File content"));
    }
}
