//! 处理教学网（服务端）给出的文件名，再用于本地路径。

/// 把服务端给出的名字（附件名、学生上传的文件名、课程或作业标题等）变成一个安全的
/// 文件名：路径分隔符、Windows 不允许的字符和控制字符替换为 `_`，因此结果不会跳出
/// 目标目录；空名、`.`、`..` 变为 `_`。
pub fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows 不允许文件名以点或空格结尾；`.` 和 `..` 也因此变为空
    let s = s.trim_start().trim_end_matches(['.', ' ']);
    if s.is_empty() {
        "_".to_owned()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_path_separators() {
        assert_eq!(sanitize("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(sanitize(r"a\b"), "a_b");
        assert_eq!(sanitize("/abs/report.pdf"), "_abs_report.pdf");
    }

    #[test]
    fn sanitize_replaces_characters_invalid_on_windows() {
        assert_eq!(sanitize("a:b*c?d\"e<f>g|h"), "a_b_c_d_e_f_g_h");
        assert_eq!(sanitize("tab\there"), "tab_here");
    }

    #[test]
    fn sanitize_keeps_normal_names() {
        assert_eq!(sanitize("实验报告 v2.pdf"), "实验报告 v2.pdf");
        assert_eq!(sanitize(".hidden"), ".hidden");
    }

    #[test]
    fn sanitize_never_returns_special_names() {
        assert_eq!(sanitize(""), "_");
        assert_eq!(sanitize("  "), "_");
        assert_eq!(sanitize("."), "_");
        assert_eq!(sanitize(".."), "_");
        // Windows 不允许结尾的点和空格
        assert_eq!(sanitize("name. "), "name");
    }
}
