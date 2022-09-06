use std::collections::HashMap;
use std::iter;

use casm::ap_change::ApplyApChange;
use itertools::zip_eq;
use sierra::edit_state::{put_results, take_args, EditStateError};
use sierra::ids::VarId;
use sierra::program::{Function, GenBranchInfo, StatementIdx};
use thiserror::Error;

use crate::invocations::BranchRefChanges;
use crate::references::{
    build_function_parameter_refs, ReferenceValue, ReferencesError, StatementRefs,
};

#[derive(Error, Debug, Eq, PartialEq)]
pub enum AnnotationError {
    #[error("Inconsistent references annotations.")]
    InconsistentReferencesAnnotation(StatementIdx),
    #[error("Inconsistent return type annotation.")]
    InconsistentReturnTypesAnnotation(StatementIdx),
    #[error("InvalidStatementIdx")]
    InvalidStatementIdx,
    #[error("MissingReferencesForStatement")]
    MissingReferencesForStatement(StatementIdx),
    #[error(transparent)]
    EditStateError(#[from] EditStateError),
    #[error(transparent)]
    ReferencesError(#[from] ReferencesError),
}

/// An annotation that specifies the expected return types at each statement.
/// This is used to propagate the return type to return statements.
/// Note that this is less strict then annotating each statement with the function it belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReturnTypesAnnotation(usize);

/// Annotation that represent the state at each program statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatementAnnotations {
    pub refs: StatementRefs,
    pub return_types: ReturnTypesAnnotation,
}

/// Annotations of the program statements.
/// See StatementAnnotations.
pub struct ProgramAnnotations {
    // Optional per statement annotation.
    per_statement_annotations: Vec<Option<StatementAnnotations>>,
}
impl ProgramAnnotations {
    fn new(n_statements: usize) -> Self {
        ProgramAnnotations {
            per_statement_annotations: iter::repeat_with(|| None).take(n_statements).collect(),
        }
    }

    // Creates a ProgramAnnotations object based on 'n_statements' and a given functions list.
    pub fn create(n_statements: usize, functions: &[Function]) -> Result<Self, AnnotationError> {
        let mut annotations = ProgramAnnotations::new(n_statements);
        let mut return_annotations = HashMap::new();
        for func in functions {
            let next_type_annotations = return_annotations.len();
            let return_annotation = return_annotations
                .entry(&func.ret_types)
                .or_insert_with(|| ReturnTypesAnnotation(next_type_annotations));

            annotations.set_or_assert(
                func.entry,
                StatementAnnotations {
                    refs: build_function_parameter_refs(func)?,
                    return_types: return_annotation.clone(),
                },
            )?
        }
        // TODO(ilya, 10/10/2022): Store Return types in ProgramAnnotations.

        Ok(annotations)
    }

    /// Sets the annotations at 'statement_id' to 'annotations'
    /// If the annotations for this statement were set previously, assert that the previous
    /// assignment is consistent with the new assignment.
    pub fn set_or_assert(
        &mut self,
        statement_id: StatementIdx,
        annotations: StatementAnnotations,
    ) -> Result<(), AnnotationError> {
        let idx = statement_id.0;
        match self.per_statement_annotations.get(idx).ok_or(AnnotationError::InvalidStatementIdx)? {
            None => self.per_statement_annotations[idx] = Some(annotations),
            Some(expected_annotations) => {
                if expected_annotations.refs != annotations.refs {
                    return Err(AnnotationError::InconsistentReferencesAnnotation(statement_id));
                }
                if expected_annotations.return_types != annotations.return_types {
                    return Err(AnnotationError::InconsistentReturnTypesAnnotation(statement_id));
                }
            }
        };
        Ok(())
    }

    /// Returns the result of applying take_args to the StatementAnnotations at statement_idx.
    /// Assumes statement_idx is a valid index.
    pub fn get_annotations_after_take_args<'a>(
        &self,
        statement_idx: StatementIdx,
        ref_ids: impl Iterator<Item = &'a VarId>,
    ) -> Result<(StatementAnnotations, Vec<ReferenceValue>), AnnotationError> {
        let statement_annotations = &self.per_statement_annotations[statement_idx.0]
            .as_ref()
            .ok_or(AnnotationError::MissingReferencesForStatement(statement_idx))?;

        let (statement_refs, taken_refs) = take_args(statement_annotations.refs.clone(), ref_ids)?;
        Ok((
            StatementAnnotations {
                refs: statement_refs,
                return_types: statement_annotations.return_types.clone(),
            },
            taken_refs,
        ))
    }

    // Propagate the annotations from the statement at 'statement_idx' to all the branches
    // from said statement.
    // 'annotations' is the result of calling get_annotations_after_take_args at
    // 'statement_idx' and 'per_branch_ref_changes' are the reference changes at each branch.
    pub fn propagate_annotations(
        &mut self,
        statement_idx: StatementIdx,
        annotations: StatementAnnotations,
        branches: &[GenBranchInfo<StatementIdx>],
        per_branch_ref_changes: impl Iterator<Item = BranchRefChanges>,
    ) -> Result<(), AnnotationError> {
        for (branch_info, branch_result) in zip_eq(branches, per_branch_ref_changes) {
            let mut new_refs: StatementRefs =
                HashMap::with_capacity(annotations.refs.len() + branch_result.refs.len());
            for (var_id, ref_value) in &annotations.refs {
                new_refs.insert(
                    var_id.clone(),
                    ReferenceValue {
                        expression: ref_value
                            .expression
                            .clone()
                            .apply_ap_change(branch_result.ap_change)
                            .map_err(ReferencesError::ApChangeError)?,
                        ty: ref_value.ty.clone(),
                    },
                );
            }

            self.set_or_assert(
                statement_idx.next(&branch_info.target),
                StatementAnnotations {
                    refs: put_results(new_refs, zip_eq(&branch_info.results, branch_result.refs))?,
                    return_types: annotations.return_types.clone(),
                },
            )?;
        }
        Ok(())
    }
}
