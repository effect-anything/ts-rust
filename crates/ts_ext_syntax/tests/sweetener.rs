use ts_ext_syntax::{ExpandRequest, Phase, expand};

fn request<'a>(filename: &'a str, code: &'a str) -> ExpandRequest<'a> {
    ExpandRequest {
        filename,
        code,
        phase: Phase::Check,
    }
}

#[test]
fn sts_reports_not_built_in() {
    let code = "let x = 1;\n";
    for filename in ["main.sts", "main.stsx", "foo.sts?import"] {
        let expansion = expand(request(filename, code));
        assert_eq!(expansion.code, code, "{filename}");
        assert!(
            expansion
                .diagnostics
                .iter()
                .any(|d| d.message.contains("Sweetener expansion is not built in yet")),
            "{filename}: {:?}",
            expansion.diagnostics
        );
    }
}

#[test]
fn ordinary_typescript_is_unchanged() {
    let code = "const n = 1 + 2;\n";
    for filename in ["main.ts", "foo.sts.ts"] {
        let expansion = expand(request(filename, code));
        assert_eq!(expansion.code, code, "{filename}");
        assert!(
            expansion
                .diagnostics
                .iter()
                .all(|d| !d.message.contains("Sweetener expansion is not built in yet")),
            "{filename}: {:?}",
            expansion.diagnostics
        );
    }
}
