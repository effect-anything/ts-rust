use ts_ext_sweetener::{ExpandRequest, Phase, expand};

fn request<'a>(filename: &'a str, code: &'a str) -> ExpandRequest<'a> {
    ExpandRequest {
        filename,
        code,
        phase: Phase::Check,
    }
}

#[test]
fn plain_sts_is_expanded_without_a_stub_diagnostic() {
    let code = "let x = 1;\n";
    for filename in ["main.sts", "main.stsx", "foo.sts?import"] {
        let expansion = expand(request(filename, code));
        assert_eq!(expansion.code, code, "{filename}");
        assert!(
            expansion.diagnostics.is_empty(),
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
            expansion.diagnostics.is_empty(),
            "{filename}: {:?}",
            expansion.diagnostics
        );
    }
}
