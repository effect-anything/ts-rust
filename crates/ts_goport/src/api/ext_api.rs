//! PORT: not in Go. API methods our fork adds for content-mapped files
//! (Sweetener `.sts`/`.stsx` and any other content mapper). `session_p1.rs`
//! `handle_request` keeps one call into [`handle`].
//!
//! The upstream API serves the *virtual* file: the AST `getSourceFile`
//! returns is the mapper's output. These methods tell a client what the
//! mapper did, following the conventions of the other methods: a request
//! names `snapshot`, `project` and `file`, a batch takes `files` or
//! `positions` and answers one entry per input, in order.
//!
//! - `getMappedFileInfo` `{snapshot, project, file}` → `MappedFileInfo | null`
//! - `getMappedFilesInfo` `{snapshot, project, files?}` → `(MappedFileInfo | null)[]`;
//!   without `files`, every content-mapped file of the program.
//! - `getOriginalPositions` `{snapshot, project, file, positions}` →
//!   `{position, fidelity}[]`: virtual → original.
//! - `getVirtualPositions` `{snapshot, project, file, positions}` →
//!   `{position, fidelity}[][]`: original → virtual. One original position
//!   can appear several times in macro output, so each entry is a list.
//!
//! Positions are UTF-16 code units, as everywhere in the API; the Rust
//! functions below ([`mapped_file_info`], [`original_positions`], ...) work
//! in UTF-8 bytes like the compiler. A file
//! no mapper produced maps every position to itself (`exact`).
//!
//! `MappedFileInfo`:
//! - `fileName`, `contentMapper`;
//! - `dependencies`: canonical paths of every file the transform read
//!   (`for syntax` macro modules and what they load), sorted;
//! - `syntaxImports`: the file's `import {...} from "m" for syntax;`
//!   declarations, which the virtual AST no longer contains;
//! - `expansions`: each macro invocation (original range) and the virtual
//!   range it produced, with a label naming the macro and its module;
//! - `regions`: the virtual text partitioned into `verbatim` (user-written,
//!   maps 1:1), `generated` (macro output attributed to an original range)
//!   and `synthesized` (no original counterpart) runs.

use crate::api::prelude::*;
use crate::ast::source_file_content_mapper_info;
use crate::ext::spanmap_ext::{Expansion, MapperInfo, SyntaxImport};
use crate::frontend::json::{
    JsonDecoder, JsonError, MarshalerTo, UnmarshalerFrom, json_unmarshal_decode,
};
use crate::frontend::json_ext::{
    AnyValue, JsonValue, marshal_field, unmarshal_root, unmarshal_struct_fields, write_object_end,
    write_object_start,
};
use crate::gostd::errors;
use crate::spanmap::{Feature, Fidelity, Kind, SpanMap};

pub const GET_MAPPED_FILE_INFO: &str = "getMappedFileInfo";
pub const GET_MAPPED_FILES_INFO: &str = "getMappedFilesInfo";
pub const GET_ORIGINAL_POSITIONS: &str = "getOriginalPositions";
pub const GET_VIRTUAL_POSITIONS: &str = "getVirtualPositions";

/// Our methods, or `None` for the upstream dispatch.
pub fn handle(
    session: &Session,
    method: &str,
    params: &JsonValue,
) -> Option<Result<Option<Box<dyn AnyValue>>, GoError>> {
    if !matches!(
        method,
        GET_MAPPED_FILE_INFO | GET_MAPPED_FILES_INFO | GET_ORIGINAL_POSITIONS | GET_VIRTUAL_POSITIONS
    ) {
        return None;
    }
    let mut parsed = MappedFileParams::default();
    if let Err(err) = unmarshal_root(&params.0, &mut parsed) {
        let err = errors::from_value(err);
        return Some(Err(errors::errorf(
            format!("{}: {}", *ERR_INVALID_REQUEST, err),
            vec![ERR_INVALID_REQUEST.clone(), err],
        )));
    }
    Some(dispatch(session, method, &parsed))
}

fn dispatch(
    session: &Session,
    method: &str,
    params: &MappedFileParams,
) -> Result<Option<Box<dyn AnyValue>>, GoError> {
    let sd = session.get_snapshot_data(params.snapshot)?;
    let program = &sd.get_program(&params.project)?;
    let base = program_base_directory(program);
    let find = |file: &DocumentIdentifier| program.get_source_file(&file.to_file_name(&base));
    match method {
        GET_MAPPED_FILE_INFO => {
            let info = params
                .file
                .as_ref()
                .and_then(|file| find(file))
                .map(|source_file| api_file_info(source_file.root));
            Ok(to_any(info))
        }
        GET_MAPPED_FILES_INFO => {
            let infos: Vec<Option<MappedFileInfo>> = match &params.files {
                Some(files) => files
                    .iter()
                    .map(|file| find(file).map(|source_file| api_file_info(source_file.root)))
                    .collect(),
                None => program
                    .get_source_files()
                    .iter()
                    .filter(|source_file| source_file_content_mapper_info(source_file.root).is_some())
                    .map(|source_file| Some(api_file_info(source_file.root)))
                    .collect(),
            };
            Ok(to_any(infos))
        }
        _ => {
            let Some(source_file) = params.file.as_ref().and_then(|file| find(file)) else {
                return Ok(to_any(None::<Vec<MappedPosition>>));
            };
            let root = source_file.root;
            let info = source_file_content_mapper_info(root);
            let map = info.and_then(|info| info.span_map.as_deref());
            let virtual_map = crate::ast::source_file_get_position_map(root);
            let original_map = info.map(|info| crate::ast::compute_position_map(&info.original_text));
            let original_map = original_map.as_ref().unwrap_or(&*virtual_map);
            if method == GET_ORIGINAL_POSITIONS {
                let input: Vec<i32> =
                    params.positions.iter().map(|&p| virtual_map.utf16_to_utf8(p)).collect();
                let mut out = original_positions(map, &input);
                for item in &mut out {
                    item.position = original_map.utf8_to_utf16(item.position);
                }
                Ok(to_any(out))
            } else {
                let input: Vec<i32> =
                    params.positions.iter().map(|&p| original_map.utf16_to_utf8(p)).collect();
                let mut out = virtual_positions(map, &input);
                for item in out.iter_mut().flatten() {
                    item.position = virtual_map.utf8_to_utf16(item.position);
                }
                Ok(to_any(out))
            }
        }
    }
}

/// One file's answer with every position in UTF-16 code units, as the API
/// reports positions.
fn api_file_info(root: Node) -> MappedFileInfo {
    let info = source_file_content_mapper_info(root);
    let mut answer = mapped_file_info(crate::ast::source_file_file_name(root), info, root.end());
    let virtual_map = crate::ast::source_file_get_position_map(root);
    let original_map = info.map(|info| crate::ast::compute_position_map(&info.original_text));
    let original_map = original_map.as_ref().unwrap_or(&*virtual_map);
    answer.convert_positions(
        &|p| virtual_map.utf8_to_utf16(p),
        &|p| original_map.utf8_to_utf16(p),
    );
    answer
}

/// Params of every method here; each reads the members it needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedFileParams {
    pub snapshot: SnapshotID,
    pub project: project::ID,
    pub file: Option<DocumentIdentifier>,
    pub files: Option<Vec<DocumentIdentifier>>,
    pub positions: Vec<i32>,
}

impl UnmarshalerFrom for MappedFileParams {
    fn unmarshal_json_from(&mut self, dec: &mut JsonDecoder<'_>) -> Result<(), JsonError> {
        let is_object = unmarshal_struct_fields(dec, "api.MappedFileParams", |name, dec| {
            match name {
                "snapshot" => json_unmarshal_decode(dec, &mut self.snapshot)?,
                "project" => json_unmarshal_decode(dec, &mut self.project)?,
                "file" => {
                    let mut file = DocumentIdentifier::default();
                    json_unmarshal_decode(dec, &mut file)?;
                    self.file = Some(file);
                }
                "files" => {
                    let mut files = Vec::new();
                    json_unmarshal_decode(dec, &mut files)?;
                    self.files = Some(files);
                }
                "positions" => json_unmarshal_decode(dec, &mut self.positions)?,
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        if !is_object {
            *self = Self::default();
        }
        Ok(())
    }
}

/// How a mapped position relates to its counterpart.
fn fidelity_name(fidelity: Fidelity) -> &'static str {
    match fidelity {
        Fidelity::EXACT => "exact",
        Fidelity::ATOM => "atom",
        Fidelity::APPROXIMATE => "approximate",
        _ => "none",
    }
}

/// One mapped position. `fidelity` is `exact` (1:1, safe for edits),
/// `atom` (inside generated code: the position of its whole original
/// range), `approximate`, or `none` (no counterpart; `position` is the
/// nearest one the map gives).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedPosition {
    pub position: i32,
    pub fidelity: &'static str,
}

impl MarshalerTo for MappedPosition {
    fn marshal_json_to(&self, enc: &mut String) -> Result<(), JsonError> {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "position", &self.position)?;
        marshal_field(enc, first, "fidelity", &self.fidelity)?;
        write_object_end(enc);
        Ok(())
    }
}

/// Virtual → original, one answer per position.
#[must_use]
pub fn original_positions(map: Option<&SpanMap>, positions: &[i32]) -> Vec<MappedPosition> {
    positions
        .iter()
        .map(|&pos| {
            let (position, fidelity) = SpanMap::virtual_to_original_position(map, pos);
            MappedPosition {
                position,
                fidelity: fidelity_name(fidelity),
            }
        })
        .collect()
}

/// Original → virtual: every virtual projection of each position, in
/// virtual order; empty for a position no segment covers.
#[must_use]
pub fn virtual_positions(map: Option<&SpanMap>, positions: &[i32]) -> Vec<Vec<MappedPosition>> {
    positions
        .iter()
        .map(|&pos| {
            SpanMap::original_to_virtual_positions(map, pos, Feature::ALL)
                .into_iter()
                .map(|mapped| MappedPosition {
                    position: mapped.position,
                    fidelity: fidelity_name(mapped.fidelity),
                })
                .collect()
        })
        .collect()
}

/// One run of the virtual text and where it came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Region {
    pub virtual_start: i32,
    pub virtual_end: i32,
    /// `verbatim`, `generated` or `synthesized`.
    pub kind: &'static str,
    /// The original range; `None` for a synthesized run.
    pub original: Option<(i32, i32)>,
}

impl MarshalerTo for Region {
    fn marshal_json_to(&self, enc: &mut String) -> Result<(), JsonError> {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "virtualStart", &self.virtual_start)?;
        marshal_field(enc, first, "virtualEnd", &self.virtual_end)?;
        marshal_field(enc, first, "kind", &self.kind)?;
        if let Some((start, end)) = self.original {
            marshal_field(enc, first, "originalStart", &start)?;
            marshal_field(enc, first, "originalEnd", &end)?;
        }
        write_object_end(enc);
        Ok(())
    }
}

/// The virtual text `[0, len)` as consecutive regions: a verbatim segment
/// is user-written text, an atom is generated text attributed to its
/// original range, and anything between segments is synthesized.
/// Adjacent runs of one kind that continue each other are merged.
#[must_use]
pub fn regions(map: Option<&SpanMap>, len: i32) -> Vec<Region> {
    let Some(map) = map else {
        return vec![Region {
            virtual_start: 0,
            virtual_end: len,
            kind: "verbatim",
            original: Some((0, len)),
        }];
    };
    let mut out: Vec<Region> = Vec::new();
    let mut push = |region: Region| {
        if region.virtual_start >= region.virtual_end {
            return;
        }
        if let Some(last) = out.last_mut()
            && last.kind == region.kind
            && last.virtual_end == region.virtual_start
        {
            match (last.original, region.original) {
                (None, None) => {
                    last.virtual_end = region.virtual_end;
                    return;
                }
                (Some((ls, le)), Some((rs, re))) if region.kind == "verbatim" && le == rs => {
                    last.virtual_end = region.virtual_end;
                    last.original = Some((ls, re));
                    return;
                }
                _ => {}
            }
        }
        out.push(region);
    };
    let mut cursor = 0;
    for segment in SpanMap::segments(Some(map)) {
        push(Region {
            virtual_start: cursor,
            virtual_end: segment.virtual_start.min(len),
            kind: "synthesized",
            original: None,
        });
        push(Region {
            virtual_start: segment.virtual_start,
            virtual_end: segment.virtual_end.min(len),
            kind: if segment.kind == Kind::VERBATIM {
                "verbatim"
            } else {
                "generated"
            },
            original: Some((segment.original_start, segment.original_end)),
        });
        cursor = cursor.max(segment.virtual_end);
    }
    push(Region {
        virtual_start: cursor,
        virtual_end: len,
        kind: "synthesized",
        original: None,
    });
    out
}

/// `Expansion` and `SyntaxImport` live in another crate of the split, so
/// they are written by these functions rather than a `MarshalerTo` impl.
fn write_expansion(item: &Expansion, enc: &mut String) -> Result<(), JsonError> {
    let first = &mut true;
    write_object_start(enc);
    marshal_field(enc, first, "virtualStart", &item.virtual_start)?;
    marshal_field(enc, first, "virtualEnd", &item.virtual_end)?;
    marshal_field(enc, first, "originalStart", &item.original_start)?;
    marshal_field(enc, first, "originalEnd", &item.original_end)?;
    marshal_field(enc, first, "label", &item.label)?;
    write_object_end(enc);
    Ok(())
}

fn write_syntax_import(item: &SyntaxImport, enc: &mut String) -> Result<(), JsonError> {
    let first = &mut true;
    write_object_start(enc);
    marshal_field(enc, first, "specifier", &item.specifier)?;
    if let Some(resolved) = &item.resolved {
        marshal_field(enc, first, "resolved", resolved)?;
    }
    write_list(enc, first, "bindings", &item.bindings, |binding, enc| {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "imported", &binding.imported)?;
        marshal_field(enc, first, "local", &binding.local)?;
        write_object_end(enc);
        Ok(())
    })?;
    marshal_field(enc, first, "shadowsCore", &item.shadows_core)?;
    marshal_field(enc, first, "originalStart", &item.original_start)?;
    marshal_field(enc, first, "originalEnd", &item.original_end)?;
    write_object_end(enc);
    Ok(())
}

fn write_list<T>(
    enc: &mut String,
    first: &mut bool,
    name: &str,
    items: &[T],
    write: fn(&T, &mut String) -> Result<(), JsonError>,
) -> Result<(), JsonError> {
    if !*first {
        enc.push(',');
    }
    *first = false;
    enc.push('"');
    enc.push_str(name);
    enc.push_str("\":[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            enc.push(',');
        }
        write(item, enc)?;
    }
    enc.push(']');
    Ok(())
}

/// What a content mapper did to one file. `mapped` is false (and the
/// mapper fields empty) for a file no mapper produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedFileInfo {
    pub file_name: String,
    pub mapped: bool,
    pub content_mapper: String,
    pub dependencies: Vec<String>,
    pub syntax_imports: Vec<SyntaxImport>,
    pub expansions: Vec<Expansion>,
    pub regions: Vec<Region>,
}

impl MarshalerTo for MappedFileInfo {
    fn marshal_json_to(&self, enc: &mut String) -> Result<(), JsonError> {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "fileName", &self.file_name)?;
        marshal_field(enc, first, "mapped", &self.mapped)?;
        marshal_field(enc, first, "contentMapper", &self.content_mapper)?;
        marshal_field(enc, first, "dependencies", &self.dependencies)?;
        write_list(enc, first, "syntaxImports", &self.syntax_imports, write_syntax_import)?;
        write_list(enc, first, "expansions", &self.expansions, write_expansion)?;
        marshal_field(enc, first, "regions", &self.regions)?;
        write_object_end(enc);
        Ok(())
    }
}

impl MappedFileInfo {
    /// Rewrites every position: `v` for virtual ones, `o` for original ones.
    pub fn convert_positions(&mut self, v: &dyn Fn(i32) -> i32, o: &dyn Fn(i32) -> i32) {
        for item in &mut self.expansions {
            item.virtual_start = v(item.virtual_start);
            item.virtual_end = v(item.virtual_end);
            item.original_start = o(item.original_start);
            item.original_end = o(item.original_end);
        }
        for item in &mut self.syntax_imports {
            item.original_start = o(item.original_start);
            item.original_end = o(item.original_end);
        }
        for item in &mut self.regions {
            item.virtual_start = v(item.virtual_start);
            item.virtual_end = v(item.virtual_end);
            item.original = item.original.map(|(start, end)| (o(start), o(end)));
        }
    }
}

/// The answer for one file, for this API and for in-process callers
/// (`virtual_len` is the virtual text's length).
#[must_use]
pub fn mapped_file_info(
    file_name: &str,
    info: Option<&crate::ast::ContentMapperFileInfo>,
    virtual_len: i32,
) -> MappedFileInfo {
    file_info_of(
        file_name,
        info.map(|info| (info.content_mapper.as_str(), info.span_map.as_deref())),
        virtual_len,
    )
}

/// [`mapped_file_info`] from the mapper's name and span map.
#[must_use]
pub fn file_info_of(
    file_name: &str,
    mapper: Option<(&str, Option<&SpanMap>)>,
    virtual_len: i32,
) -> MappedFileInfo {
    let Some((content_mapper, map)) = mapper else {
        return MappedFileInfo {
            file_name: file_name.to_owned(),
            regions: regions(None, virtual_len),
            ..MappedFileInfo::default()
        };
    };
    let MapperInfo {
        expansions,
        dependencies,
        syntax_imports,
    } = SpanMap::mapper_info(map).clone();
    MappedFileInfo {
        file_name: file_name.to_owned(),
        mapped: true,
        content_mapper: content_mapper.to_owned(),
        dependencies,
        syntax_imports,
        expansions,
        regions: regions(map, virtual_len),
    }
}

#[cfg(test)]
#[path = "ext_api_tests.rs"]
mod tests;
