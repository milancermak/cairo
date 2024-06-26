use std::ops::Add;

use cairo_lang_defs::db::DefsGroup;
use cairo_lang_defs::diagnostic_utils::StableLocation;
use cairo_lang_diagnostics::ToOption;
use cairo_lang_filesystem::ids::{FileId, FileLongId, VirtualFile};
use cairo_lang_sierra::program::StatementIdx;
use cairo_lang_syntax::node::{Terminal, TypedSyntaxNode};
use cairo_lang_utils::unordered_hash_map::UnorderedHashMap;
use itertools::Itertools;

#[cfg(test)]
#[path = "statements_locations_test.rs"]
mod test;

/// Returns an identifier of the function that contains the given [StableLocation].
/// It is a fully qualified path to the function which contains:
/// - fully qualified path to the file module,
/// - relative path to the function in the file module.
pub fn containing_function_identifier(
    db: &dyn DefsGroup,
    location: StableLocation,
) -> Option<String> {
    let file_id = location.file_id(db.upcast());
    let absolute_semantic_path_to_file_module = file_module_absolute_identifier(db, file_id)?;

    let relative_semantic_path = function_identifier_relative_to_file_module(db, location);
    Some(absolute_semantic_path_to_file_module.add("::").add(&relative_semantic_path))
}

/// Returns an identifier of the function that contains the given [StableLocation].
/// It is a fully qualified path to the function which contains:
/// - fully qualified path to the file module,
/// - relative path to the function in the file module.
///
/// In case the fully qualified path to the file module cannot be found
/// it is replaced in the fully qualified function path by the file name.
pub fn containing_function_identifier_for_tests(
    db: &dyn DefsGroup,
    location: StableLocation,
) -> String {
    let file_id = location.file_id(db.upcast());
    let absolute_semantic_path_to_file_module = file_module_absolute_identifier(db, file_id)
        .unwrap_or_else(|| file_id.file_name(db.upcast()));

    let relative_semantic_path = function_identifier_relative_to_file_module(db, location);
    absolute_semantic_path_to_file_module.add("::").add(&relative_semantic_path)
}

/// Returns the path (modules and impls) to the function in the file.
/// The path is relative to the file module.
pub fn function_identifier_relative_to_file_module(
    db: &dyn DefsGroup,
    location: StableLocation,
) -> String {
    let syntax_db = db.upcast();
    let mut relative_semantic_path_segments: Vec<String> = vec![];
    let mut syntax_node = location.syntax_node(db);
    loop {
        // TODO(Gil): Extract this function into a trait of syntax kind to support future
        // function containing items (specifically trait functions).
        match syntax_node.kind(syntax_db) {
            cairo_lang_syntax::node::kind::SyntaxKind::FunctionWithBody => {
                let function_name =
                    cairo_lang_syntax::node::ast::FunctionWithBody::from_syntax_node(
                        syntax_db,
                        syntax_node.clone(),
                    )
                    .declaration(syntax_db)
                    .name(syntax_db)
                    .text(syntax_db);
                relative_semantic_path_segments.push(function_name.to_string());
            }
            cairo_lang_syntax::node::kind::SyntaxKind::ItemImpl => {
                let impl_name = cairo_lang_syntax::node::ast::ItemImpl::from_syntax_node(
                    syntax_db,
                    syntax_node.clone(),
                )
                .name(syntax_db)
                .text(syntax_db);
                relative_semantic_path_segments.push(impl_name.to_string());
            }
            cairo_lang_syntax::node::kind::SyntaxKind::ItemModule => {
                let module_name = cairo_lang_syntax::node::ast::ItemModule::from_syntax_node(
                    syntax_db,
                    syntax_node.clone(),
                )
                .name(syntax_db)
                .text(syntax_db);
                relative_semantic_path_segments.push(module_name.to_string());
            }
            _ => {}
        }
        if let Some(parent) = syntax_node.parent() {
            syntax_node = parent;
        } else {
            break;
        }
    }

    relative_semantic_path_segments.into_iter().rev().join("::")
}

/// This function returns a fully qualified path to the file module.
/// `None` should be returned only for compiler tests where files of type `VirtualFile` may be non
/// generated files.
pub fn file_module_absolute_identifier(db: &dyn DefsGroup, mut file_id: FileId) -> Option<String> {
    // `VirtualFile` is a generated file (e.g., by macros like `#[starknet::contract]`)
    // that won't have a matching file module in the db. Instead, we find its non generated parent
    // which is in the same module and have a matching file module in the db.
    while let FileLongId::Virtual(VirtualFile { parent: Some(parent), .. }) =
        db.lookup_intern_file(file_id)
    {
        file_id = parent;
    }

    let file_modules = db.file_modules(file_id).to_option()?;
    let full_path = file_modules.first().unwrap().full_path(db.upcast());

    Some(full_path)
}

/// The location of the Cairo source code which caused a statement to be generated.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StatementsLocations {
    pub locations: UnorderedHashMap<StatementIdx, StableLocation>,
}

impl StatementsLocations {
    /// Creates a new [StatementsLocations] object from a list of [`Option<StableLocation>`].
    pub fn from_locations_vec(locations_vec: &[Option<StableLocation>]) -> Self {
        let mut locations = UnorderedHashMap::default();
        for (idx, location) in locations_vec.iter().enumerate() {
            if let Some(location) = location {
                locations.insert(StatementIdx(idx), *location);
            }
        }
        Self { locations }
    }
    /// Builds a map between each Sierra statement index and a string representation of the Cairo
    /// function that it was generated from. The function representation is composed of the function
    /// name and the path (modules and impls) to the function in the file. It is used for places
    /// without db access such as the profiler.
    // TODO(Gil): Add a db access to the profiler and remove this function.
    pub fn get_statements_functions_map(
        &self,
        db: &dyn DefsGroup,
    ) -> UnorderedHashMap<StatementIdx, String> {
        self.locations.map(|s| containing_function_identifier_for_tests(db, *s))
    }
}
