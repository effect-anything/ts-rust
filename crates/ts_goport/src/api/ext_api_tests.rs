use super::*;
use crate::ext::spanmap_ext::SyntaxBinding;
use crate::spanmap::{Segment, new};

fn seg(vs: i32, ve: i32, os: i32, oe: i32, kind: Kind) -> Segment {
    Segment {
        virtual_start: vs,
        virtual_end: ve,
        original_start: os,
        original_end: oe,
        kind,
        features: Feature::ALL,
    }
}

/// `import {twice} from "./m.sts" for syntax;\nconst a = twice(v);`
/// expanded to `\nconst a = [v, v];` -- the import (0..43) is consumed,
/// `const a = ` is user text, `[`, `, `, `]` are generated and both `v`
/// copy the argument.
fn sample() -> SpanMap {
    new(&[
        seg(0, 11, 43, 54, Kind::VERBATIM),
        seg(11, 12, 54, 62, Kind::ATOM),
        seg(12, 13, 60, 61, Kind::VERBATIM),
        seg(13, 15, 54, 62, Kind::ATOM),
        seg(15, 16, 60, 61, Kind::VERBATIM),
        seg(16, 17, 54, 62, Kind::ATOM),
    ])
    .with_mapper_info(MapperInfo {
        expansions: vec![Expansion {
            virtual_start: 11,
            virtual_end: 17,
            original_start: 54,
            original_end: 62,
            label: "macro `twice` from \"./m.sts\"".into(),
        }],
        dependencies: vec!["/p/m.sts".into()],
        syntax_imports: vec![SyntaxImport {
            specifier: "./m.sts".into(),
            resolved: Some("/p/m.sts".into()),
            bindings: vec![SyntaxBinding {
                imported: "twice".into(),
                local: "twice".into(),
            }],
            shadows_core: false,
            original_start: 0,
            original_end: 43,
        }],
    })
}

#[test]
fn params_decode_single_and_batch() {
    let mut params = MappedFileParams::default();
    unmarshal_root(
        br#"{"snapshot":3,"project":"p","file":"/a.sts","positions":[1,5]}"#,
        &mut params,
    )
    .unwrap();
    assert_eq!(params.positions, vec![1, 5]);
    assert!(params.file.is_some() && params.files.is_none());
    let mut params = MappedFileParams::default();
    unmarshal_root(br#"{"snapshot":3,"project":"p","files":["/a.sts","/b.ts"]}"#, &mut params)
        .unwrap();
    assert_eq!(params.files.map(|f| f.len()), Some(2));
}

#[test]
fn regions_partition_the_virtual_text() {
    let map = sample();
    let got: Vec<_> = regions(Some(&map), 19)
        .into_iter()
        .map(|r| (r.virtual_start, r.virtual_end, r.kind, r.original))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, 11, "verbatim", Some((43, 54))),
            (11, 12, "generated", Some((54, 62))),
            (12, 13, "verbatim", Some((60, 61))),
            (13, 15, "generated", Some((54, 62))),
            (15, 16, "verbatim", Some((60, 61))),
            (16, 17, "generated", Some((54, 62))),
            (17, 19, "synthesized", None),
        ]
    );
    // Merged: two chained verbatim runs and a leading gap.
    let map = new(&[seg(2, 4, 10, 12, Kind::VERBATIM), seg(4, 6, 12, 14, Kind::VERBATIM)]);
    let got: Vec<_> = regions(Some(&map), 6)
        .into_iter()
        .map(|r| (r.virtual_start, r.virtual_end, r.kind))
        .collect();
    assert_eq!(got, vec![(0, 2, "synthesized"), (2, 6, "verbatim")]);
    // No map: the identity.
    assert_eq!(regions(None, 5)[0].original, Some((0, 5)));
}

#[test]
fn positions_map_both_ways() {
    let map = sample();
    let to_original = original_positions(Some(&map), &[3, 12, 14, 18]);
    let got: Vec<_> = to_original.iter().map(|p| (p.position, p.fidelity)).collect();
    assert_eq!(got[0], (46, "exact"));
    assert_eq!(got[1], (60, "exact"));
    assert_eq!(got[2].1, "atom");
    assert_eq!(got[3].1, "none");
    // `v` at original 60 appears twice in the output.
    let to_virtual = virtual_positions(Some(&map), &[46, 60, 10]);
    assert_eq!(to_virtual[0].iter().map(|p| p.position).collect::<Vec<_>>(), vec![3]);
    let v: Vec<_> = to_virtual[1]
        .iter()
        .filter(|p| p.fidelity == "exact")
        .map(|p| p.position)
        .collect();
    assert_eq!(v, vec![12, 15]);
    assert!(to_virtual[2].is_empty(), "inside the consumed import");
    // Unmapped: identity.
    assert_eq!(original_positions(None, &[7])[0], MappedPosition { position: 7, fidelity: "exact" });
    assert_eq!(virtual_positions(None, &[7])[0][0].position, 7);
}

#[test]
fn file_info_json_shape() {
    let map = sample();
    let answer = file_info_of("/p/a.sts", Some(("@effect-x/tsc", Some(&map))), 17);
    let mut out = String::new();
    answer.marshal_json_to(&mut out).unwrap();
    assert!(out.starts_with(r#"{"fileName":"/p/a.sts","mapped":true,"contentMapper":"@effect-x/tsc","dependencies":["/p/m.sts"],"syntaxImports":[{"specifier":"./m.sts","resolved":"/p/m.sts","bindings":[{"imported":"twice","local":"twice"}],"shadowsCore":false,"originalStart":0,"originalEnd":43}],"expansions":[{"virtualStart":11,"virtualEnd":17,"originalStart":54,"originalEnd":62,"label":"macro `twice` from \"./m.sts\""}],"regions":[{"virtualStart":0,"virtualEnd":11,"kind":"verbatim","originalStart":43,"originalEnd":54}"#), "{out}");
    let plain = file_info_of("/p/b.ts", None, 4);
    let mut out = String::new();
    plain.marshal_json_to(&mut out).unwrap();
    assert_eq!(
        out,
        r#"{"fileName":"/p/b.ts","mapped":false,"contentMapper":"","dependencies":[],"syntaxImports":[],"expansions":[],"regions":[{"virtualStart":0,"virtualEnd":4,"kind":"verbatim","originalStart":0,"originalEnd":4}]}"#
    );
}
