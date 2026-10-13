//! PORT: ts-rust extension (no Go counterpart). Language features over a
//! macro-expanding content mapper: hover says what macro produced the code
//! and shows it, and `custom/textDocument/showExpansion` returns it.

use std::rc::Rc;

use ts_goport::lsp::lsproto;
use ts_goport::project::{self, Session};

use super::projecttestutil::{self, TypingsInstallerOptions, files};
use super::util::*;
use crate::support::contentmappertest;

const SOURCE: &str = "export const value = 1;\nexport const pair = twice(value);\n";
const SOURCE_URI: &str = "file:///home/project/app.sw";

fn session() -> Rc<Session> {
    let mapper = contentmappertest::package_json(contentmappertest::EXPANDING_MAPPER);
    let (mut init, _utils) = projecttestutil::get_session_init_options(
        files(&[
            (
                "/home/project/tsconfig.json",
                r#"{
			"compilerOptions": { "target": "es2020", "module": "esnext", "moduleResolution": "bundler", "strict": true },
			"contentMappers": [ { "package": "mapper", "extensions": [".sw"] } ]
		}"#,
            ),
            (
                "/home/project/node_modules/mapper/package.json",
                mapper.as_str(),
            ),
            ("/home/project/app.sw", SOURCE),
        ]),
        Some(project::SessionOptions {
            watch_enabled: false,
            logging_enabled: false,
            run_external_code: true,
            ..projecttestutil::session_options("/home/project")
        }),
        TypingsInstallerOptions::default(),
    );
    init.spawner = Some(contentmappertest::new_spawner());
    project::new_session(&init)
}

fn markdown(hover: &lsproto::HoverOrNull) -> String {
    hover
        .hover
        .as_ref()
        .and_then(|hover| hover.contents.markup_content.as_ref())
        .map(|markup| markup.value.clone())
        .unwrap_or_default()
}

child_test! {
    fn hover_and_show_expansion_name_the_macro_and_its_output() {
        let session = session();
        open_kind(&session, SOURCE_URI, SOURCE, lsproto::LanguageKind("sw".into()));
        let service = session
            .get_language_service(&projecttestutil::with_request_id(&bg()), &uri(SOURCE_URI))
            .unwrap_or_else(|err| panic!("{}", err.error()));
        // On `twice` (line 1, character 22): synthesized code of the call.
        let position = lsproto::Position { line: 1, character: 22 };
        let expansion = service
            .expansion_at(&uri(SOURCE_URI), position)
            .expect("an expansion at the invocation");
        assert_eq!(expansion.label, "macro `twice`");
        assert_eq!(expansion.text, "[value, value]");
        assert_eq!(expansion.range.start, lsproto::Position { line: 1, character: 20 });
        assert_eq!(expansion.range.end, lsproto::Position { line: 1, character: 32 });
        let hover = service
            .provide_hover(
                &projecttestutil::with_request_id(&bg()),
                &lsproto::HoverParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: uri(SOURCE_URI) },
                    position: lsproto::Position { line: 1, character: 27 },
                    ..Default::default()
                },
            )
            .unwrap_or_else(|err| panic!("{}", err.error()));
        let text = markdown(&hover);
        assert!(text.contains("Expanded from macro `twice`"), "{text}");
        assert!(text.contains("[value, value]"), "{text}");
        // Outside the invocation there is no expansion.
        assert!(
            service
                .expansion_at(&uri(SOURCE_URI), lsproto::Position { line: 0, character: 14 })
                .is_none()
        );
        session.close();
    }
}
