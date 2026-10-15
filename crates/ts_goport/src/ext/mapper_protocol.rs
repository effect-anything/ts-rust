//! PORT: not in Go. Content mapper protocol additions on a mapped output
//! ([`MappedOutputExt`]): `expansions`, `dependencies` (the files a
//! transform read, e.g. `for syntax` macro modules) and `syntaxImports`; and
//! `category` / `relatedInformation` on a mapper diagnostic.
//! `contentmapper/hostimpl.rs` keeps a field and a call each.

use crate::contentmapper::prelude::*;
use crate::core::Diagnostic;
use crate::ext::spanmap_ext::{Expansion, MapperInfo, SyntaxBinding, SyntaxImport};
use crate::frontend::json_ext::{
    marshal_field, unmarshal_struct_fields, write_object_end, write_object_start,
};
use crate::gostd::GoError;
use crate::options_json::marshal_field_omitempty;
use crate::spanmap::SpanMap;
use std::sync::Arc;

/// One macro invocation and the virtual text it produced, in the selected
/// position encoding.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpansionInformation {
    pub virtual_start: i32,
    pub virtual_length: i32,
    pub original_start: i32,
    pub original_length: i32,
    pub label: String,
}

impl MarshalerTo for ExpansionInformation {
    fn marshal_json_to(&self, enc: &mut String) -> std::result::Result<(), JsonError> {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "virtualStart", &self.virtual_start)?;
        marshal_field(enc, first, "virtualLength", &self.virtual_length)?;
        marshal_field(enc, first, "originalStart", &self.original_start)?;
        marshal_field(enc, first, "originalLength", &self.original_length)?;
        marshal_field(enc, first, "label", &self.label)?;
        write_object_end(enc);
        Ok(())
    }
}

impl UnmarshalerFrom for ExpansionInformation {
    fn unmarshal_json_from(
        &mut self,
        dec: &mut JsonDecoder<'_>,
    ) -> std::result::Result<(), JsonError> {
        let is_object =
            unmarshal_struct_fields(dec, "contentmapper.ExpansionInformation", |name, dec| {
                match name {
                    "virtualStart" => json_unmarshal_decode(dec, &mut self.virtual_start)?,
                    "virtualLength" => json_unmarshal_decode(dec, &mut self.virtual_length)?,
                    "originalStart" => json_unmarshal_decode(dec, &mut self.original_start)?,
                    "originalLength" => json_unmarshal_decode(dec, &mut self.original_length)?,
                    "label" => json_unmarshal_decode(dec, &mut self.label)?,
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
        if !is_object {
            *self = ExpansionInformation::default();
        }
        Ok(())
    }
}

/// One `for syntax` import of the original file, in the selected position
/// encoding: `{specifier, resolved?, bindings: [{imported, local}],
/// shadowsCore?, originalStart, originalLength}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyntaxImportInformation {
    pub specifier: String,
    pub resolved: Option<String>,
    pub bindings: Vec<SyntaxBindingInformation>,
    pub shadows_core: bool,
    pub original_start: i32,
    pub original_length: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyntaxBindingInformation {
    pub imported: String,
    pub local: String,
}

impl MarshalerTo for SyntaxBindingInformation {
    fn marshal_json_to(&self, enc: &mut String) -> std::result::Result<(), JsonError> {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "imported", &self.imported)?;
        marshal_field(enc, first, "local", &self.local)?;
        write_object_end(enc);
        Ok(())
    }
}

impl UnmarshalerFrom for SyntaxBindingInformation {
    fn unmarshal_json_from(
        &mut self,
        dec: &mut JsonDecoder<'_>,
    ) -> std::result::Result<(), JsonError> {
        let is_object =
            unmarshal_struct_fields(dec, "contentmapper.SyntaxBindingInformation", |name, dec| {
                match name {
                    "imported" => json_unmarshal_decode(dec, &mut self.imported)?,
                    "local" => json_unmarshal_decode(dec, &mut self.local)?,
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
        if !is_object {
            *self = SyntaxBindingInformation::default();
        }
        Ok(())
    }
}

impl MarshalerTo for SyntaxImportInformation {
    fn marshal_json_to(&self, enc: &mut String) -> std::result::Result<(), JsonError> {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "specifier", &self.specifier)?;
        if let Some(resolved) = &self.resolved {
            marshal_field(enc, first, "resolved", resolved)?;
        }
        marshal_field(enc, first, "bindings", &self.bindings)?;
        if self.shadows_core {
            marshal_field(enc, first, "shadowsCore", &true)?;
        }
        marshal_field(enc, first, "originalStart", &self.original_start)?;
        marshal_field(enc, first, "originalLength", &self.original_length)?;
        write_object_end(enc);
        Ok(())
    }
}

impl UnmarshalerFrom for SyntaxImportInformation {
    fn unmarshal_json_from(
        &mut self,
        dec: &mut JsonDecoder<'_>,
    ) -> std::result::Result<(), JsonError> {
        let is_object =
            unmarshal_struct_fields(dec, "contentmapper.SyntaxImportInformation", |name, dec| {
                match name {
                    "specifier" => json_unmarshal_decode(dec, &mut self.specifier)?,
                    "resolved" => {
                        let mut resolved = String::new();
                        json_unmarshal_decode(dec, &mut resolved)?;
                        self.resolved = Some(resolved);
                    }
                    "bindings" => json_unmarshal_decode(dec, &mut self.bindings)?,
                    "shadowsCore" => json_unmarshal_decode(dec, &mut self.shadows_core)?,
                    "originalStart" => json_unmarshal_decode(dec, &mut self.original_start)?,
                    "originalLength" => json_unmarshal_decode(dec, &mut self.original_length)?,
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
        if !is_object {
            *self = SyntaxImportInformation::default();
        }
        Ok(())
    }
}

/// Our members of a mapped output. All are optional on the wire.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedOutputExt {
    pub expansions: Vec<ExpansionInformation>,
    pub dependencies: Vec<String>,
    pub syntax_imports: Vec<SyntaxImportInformation>,
}

impl MappedOutputExt {
    /// Writes the members that are present.
    pub fn marshal_fields(
        &self,
        enc: &mut String,
        first: &mut bool,
    ) -> std::result::Result<(), JsonError> {
        marshal_field_omitempty(enc, first, "expansions", &self.expansions)?;
        marshal_field_omitempty(enc, first, "dependencies", &self.dependencies)?;
        marshal_field_omitempty(enc, first, "syntaxImports", &self.syntax_imports)
    }

    /// Decodes one of our members; false for a name that is not ours.
    pub fn decode_field(
        &mut self,
        name: &str,
        dec: &mut JsonDecoder<'_>,
    ) -> std::result::Result<bool, JsonError> {
        match name {
            "expansions" => json_unmarshal_decode(dec, &mut self.expansions)?,
            "dependencies" => json_unmarshal_decode(dec, &mut self.dependencies)?,
            "syntaxImports" => json_unmarshal_decode(dec, &mut self.syntax_imports)?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// One `relatedInformation` entry of a mapper diagnostic, in original-source
/// coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelatedInformation {
    pub message_text: String,
    pub start: i32,
    pub length: i32,
}

impl MarshalerTo for RelatedInformation {
    fn marshal_json_to(&self, enc: &mut String) -> std::result::Result<(), JsonError> {
        let first = &mut true;
        write_object_start(enc);
        marshal_field(enc, first, "messageText", &self.message_text)?;
        marshal_field(enc, first, "start", &self.start)?;
        marshal_field(enc, first, "length", &self.length)?;
        write_object_end(enc);
        Ok(())
    }
}

impl UnmarshalerFrom for RelatedInformation {
    fn unmarshal_json_from(
        &mut self,
        dec: &mut JsonDecoder<'_>,
    ) -> std::result::Result<(), JsonError> {
        let is_object =
            unmarshal_struct_fields(dec, "contentmapper.RelatedInformation", |name, dec| {
                match name {
                    "messageText" => json_unmarshal_decode(dec, &mut self.message_text)?,
                    "start" => json_unmarshal_decode(dec, &mut self.start)?,
                    "length" => json_unmarshal_decode(dec, &mut self.length)?,
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
        if !is_object {
            *self = RelatedInformation::default();
        }
        Ok(())
    }
}

/// The extra members of a mapper diagnostic: an optional TypeScript
/// `DiagnosticCategory` (0 warning, 1 error, 2 suggestion, 3 message; absent
/// means error, as in Go) and secondary locations in the original content.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiagnosticExt {
    pub category: Option<i32>,
    pub related_information: Vec<RelatedInformation>,
}

impl DiagnosticExt {
    /// Writes the members that are present.
    pub fn marshal_fields(
        &self,
        enc: &mut String,
        first: &mut bool,
    ) -> std::result::Result<(), JsonError> {
        if let Some(category) = &self.category {
            marshal_field(enc, first, "category", category)?;
        }
        marshal_field_omitempty(enc, first, "relatedInformation", &self.related_information)
    }

    /// Decodes `category` or `relatedInformation`.
    pub fn decode_field(
        &mut self,
        name: &str,
        dec: &mut JsonDecoder<'_>,
    ) -> std::result::Result<(), JsonError> {
        if name == "category" {
            let mut category = 0i32;
            json_unmarshal_decode(dec, &mut category)?;
            self.category = Some(category);
            return Ok(());
        }
        json_unmarshal_decode(dec, &mut self.related_information)
    }

    /// The category to report; anything absent or unknown stays an error.
    #[must_use]
    pub fn category(&self) -> crate::diagnostics::Category {
        use crate::diagnostics::Category;
        match self.category {
            Some(0) => Category::Warning,
            Some(2) => Category::Suggestion,
            Some(3) => Category::Message,
            _ => Category::Error,
        }
    }

    /// Applies the category and the related locations to the diagnostic the
    /// host just decoded. `normalize` maps an original position to UTF-8; a
    /// related location that does not map is dropped, not an error.
    pub fn apply(
        &self,
        diagnostic: Option<&mut Diagnostic>,
        normalize: impl Fn(i32) -> std::result::Result<i32, GoError>,
    ) {
        let Some(diagnostic) = diagnostic else {
            return;
        };
        diagnostic.set_category(self.category());
        let (source, code) = (diagnostic.source().to_string(), diagnostic.code());
        let mut related = Vec::with_capacity(self.related_information.len());
        for item in &self.related_information {
            if item.start < 0 || item.length < 0 || item.start > i32::MAX - item.length {
                continue;
            }
            let (Some(start), Some(end)) = (
                normalize(item.start).ok(),
                normalize(item.start + item.length).ok(),
            ) else {
                continue;
            };
            related.push(crate::ast::new_external_diagnostic(
                Node::NIL,
                TextRange::new(start, end),
                &source,
                crate::diagnostics::Category::Message,
                code,
                &item.message_text,
            ));
        }
        if !related.is_empty() {
            diagnostic.set_related_info(related);
        }
    }
}

/// Attaches the output's report, positions normalized to UTF-8 like the
/// segments, to the freshly decoded span map. An entry with a position
/// outside its text is dropped: it is information for language features
/// and tools, not part of the mapping contract.
///
/// The map is normally uniquely owned here. If it is already shared, a
/// copy carries the report: dropping it silently would hide every
/// expansion and dependency of the file.
pub fn attach_mapper_info(
    mappings: &mut Option<Arc<SpanMap>>,
    output: &MappedOutputExt,
    virtual_pos: impl Fn(i32) -> std::result::Result<i32, GoError>,
    original_pos: impl Fn(i32) -> std::result::Result<i32, GoError>,
) {
    let info = mapper_info(output, virtual_pos, original_pos);
    if info.is_empty() {
        return;
    }
    let Some(map) = mappings.take() else {
        return;
    };
    let owned = Arc::try_unwrap(map).unwrap_or_else(|shared| shared.copied());
    *mappings = Some(Arc::new(owned.with_mapper_info(info)));
}

/// The normalized report of one mapped output.
pub fn mapper_info(
    output: &MappedOutputExt,
    virtual_pos: impl Fn(i32) -> std::result::Result<i32, GoError>,
    original_pos: impl Fn(i32) -> std::result::Result<i32, GoError>,
) -> MapperInfo {
    let range = |start: i32, length: i32, f: &dyn Fn(i32) -> std::result::Result<i32, GoError>| {
        if start < 0 || length < 0 || start > i32::MAX - length {
            return None;
        }
        Some((f(start).ok()?, f(start + length).ok()?))
    };
    let expansions = output
        .expansions
        .iter()
        .filter_map(|item| {
            let (virtual_start, virtual_end) =
                range(item.virtual_start, item.virtual_length, &virtual_pos)?;
            let (original_start, original_end) =
                range(item.original_start, item.original_length, &original_pos)?;
            Some(Expansion {
                virtual_start,
                virtual_end,
                original_start,
                original_end,
                label: item.label.clone(),
            })
        })
        .collect();
    let syntax_imports = output
        .syntax_imports
        .iter()
        .filter_map(|item| {
            let (original_start, original_end) =
                range(item.original_start, item.original_length, &original_pos)?;
            Some(SyntaxImport {
                specifier: item.specifier.clone(),
                resolved: item.resolved.clone(),
                bindings: item
                    .bindings
                    .iter()
                    .map(|b| SyntaxBinding {
                        imported: b.imported.clone(),
                        local: b.local.clone(),
                    })
                    .collect(),
                shadows_core: item.shadows_core,
                original_start,
                original_end,
            })
        })
        .collect();
    let mut dependencies = output.dependencies.clone();
    dependencies.sort();
    dependencies.dedup();
    MapperInfo {
        expansions,
        dependencies,
        syntax_imports,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contentmapper::hostimpl::{Diagnostic as MapperDiagnostic, MappedOutput};

    #[test]
    fn diagnostic_related_information_round_trips() {
        let input = br#"{"messageText":"m","start":1,"length":2,"code":4001,"category":1,"relatedInformation":[{"messageText":"r","start":5,"length":3}]}"#;
        let mut diagnostic = MapperDiagnostic::default();
        crate::frontend::json::json_unmarshal(input, &mut diagnostic, &[]).unwrap();
        assert_eq!(diagnostic.ext.category, Some(1));
        assert_eq!(
            diagnostic.ext.related_information,
            vec![RelatedInformation {
                message_text: "r".to_string(),
                start: 5,
                length: 3
            }]
        );
        let mut out = String::new();
        diagnostic.marshal_json_to(&mut out).unwrap();
        assert!(
            out.contains(r#""relatedInformation":[{"messageText":"r","start":5,"length":3}]"#),
            "{out}"
        );
    }

    #[test]
    fn mapper_diagnostic_category_defaults_to_error() {
        use crate::diagnostics::Category;
        let of = |category| {
            DiagnosticExt {
                category,
                ..Default::default()
            }
            .category()
        };
        assert_eq!(of(None), Category::Error);
        assert_eq!(of(Some(1)), Category::Error);
        assert_eq!(of(Some(0)), Category::Warning);
        assert_eq!(of(Some(9)), Category::Error);
    }

    #[test]
    fn expansions_decode_and_attach_to_the_span_map() {
        // `twice(value)` at 10..22 expanded to `[value, value]` at 10..24.
        let input = br#"{"text":"const a = [value, value];","extension":".ts","mappings":[[0,10,0,10,0],[10,14,10,12,1]],"expansions":[{"virtualStart":10,"virtualLength":14,"originalStart":10,"originalLength":12,"label":"macro `twice`"}]}"#;
        let mut output = MappedOutput::default();
        crate::frontend::json::json_unmarshal(input, &mut output, &[]).unwrap();
        assert_eq!(output.ext.expansions.len(), 1);
        let mut encoded = String::new();
        output.marshal_json_to(&mut encoded).unwrap();
        assert!(
            encoded.contains(r#""expansions":[{"virtualStart":10"#),
            "{encoded}"
        );
        let mut mappings = Some(Arc::new(
            crate::spanmap::unmarshal(&output.mappings.0).unwrap(),
        ));
        attach_mapper_info(&mut mappings, &output.ext, Ok, Ok);
        let map = mappings.as_deref();
        let inside = SpanMap::expansions_containing_virtual(map, TextRange::new(11, 16));
        assert_eq!(inside.len(), 1);
        assert_eq!(inside[0].label, "macro `twice`");
        assert_eq!((inside[0].original_start, inside[0].original_end), (10, 22));
        assert!(SpanMap::expansions_containing_virtual(map, TextRange::new(0, 5)).is_empty());
        assert_eq!(SpanMap::expansions_at_original(map, 12).len(), 1);
    }

    #[test]
    fn dependencies_decode_and_attach_to_the_span_map() {
        let input = br#"{"text":"const a = 1;","extension":".ts","mappings":[[0,12,0,12,0]],"dependencies":["/p/macros.sts"]}"#;
        let mut output = MappedOutput::default();
        crate::frontend::json::json_unmarshal(input, &mut output, &[]).unwrap();
        assert_eq!(output.ext.dependencies, vec!["/p/macros.sts".to_string()]);
        let mut encoded = String::new();
        output.marshal_json_to(&mut encoded).unwrap();
        assert!(encoded.contains(r#""dependencies":["/p/macros.sts"]"#), "{encoded}");
        let mut mappings = Some(Arc::new(
            crate::spanmap::unmarshal(&output.mappings.0).unwrap(),
        ));
        attach_mapper_info(&mut mappings, &output.ext, Ok, Ok);
        assert_eq!(SpanMap::dependencies(mappings.as_deref()), ["/p/macros.sts"]);
    }

    #[test]
    fn syntax_imports_decode_and_attach_to_the_span_map() {
        let input = br#"{"text":"x;","extension":".ts","mappings":[[0,2,50,2,0]],"syntaxImports":[{"specifier":"./m.sts","resolved":"/p/m.sts","bindings":[{"imported":"twice","local":"t"}],"shadowsCore":true,"originalStart":0,"originalLength":48}]}"#;
        let mut output = MappedOutput::default();
        crate::frontend::json::json_unmarshal(input, &mut output, &[]).unwrap();
        let mut encoded = String::new();
        output.marshal_json_to(&mut encoded).unwrap();
        assert!(
            encoded.contains(r#""syntaxImports":[{"specifier":"./m.sts","resolved":"/p/m.sts","bindings":[{"imported":"twice","local":"t"}],"shadowsCore":true,"originalStart":0,"originalLength":48}]"#),
            "{encoded}"
        );
        let mut mappings = Some(Arc::new(
            crate::spanmap::unmarshal(&output.mappings.0).unwrap(),
        ));
        attach_mapper_info(&mut mappings, &output.ext, Ok, Ok);
        let imports = SpanMap::syntax_imports(mappings.as_deref());
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].resolved.as_deref(), Some("/p/m.sts"));
        assert_eq!(imports[0].bindings[0].local, "t");
        assert_eq!((imports[0].original_start, imports[0].original_end), (0, 48));
    }

    /// The report used to be dropped when the decoded map was already
    /// shared (`Arc::try_unwrap` failed). It is attached to a copy now.
    #[test]
    fn a_shared_map_still_gets_the_report() {
        let map = Arc::new(crate::spanmap::unmarshal(b"[[0,3,0,3,0]]").unwrap());
        let keep = map.clone();
        let mut mappings = Some(map);
        let ext = MappedOutputExt {
            dependencies: vec!["/p/b.sts".into(), "/p/a.sts".into(), "/p/a.sts".into()],
            ..Default::default()
        };
        attach_mapper_info(&mut mappings, &ext, Ok, Ok);
        let attached = mappings.as_deref();
        assert_eq!(SpanMap::dependencies(attached), ["/p/a.sts", "/p/b.sts"]);
        assert_eq!(SpanMap::segments(attached), SpanMap::segments(Some(&keep)));
        assert!(SpanMap::dependencies(Some(&keep)).is_empty());
    }

    #[test]
    fn out_of_range_entries_are_dropped() {
        let ext = MappedOutputExt {
            expansions: vec![ExpansionInformation {
                virtual_start: -1,
                virtual_length: 2,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(mapper_info(&ext, Ok, Ok).is_empty());
    }
}
