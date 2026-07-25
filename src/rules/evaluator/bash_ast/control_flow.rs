use brush_parser::ast::{
    AndOr, CaseClauseCommand, CaseItemPostAction, CompoundList, IfClauseCommand,
    WhileOrUntilClauseCommand,
};

use super::{CommandCollector, DYNAMIC_SHELL_WORD};

impl CommandCollector {
    pub(super) fn collect_list(&mut self, list: &CompoundList) -> Result<(), String> {
        for item in &list.0 {
            if self.execution_terminated {
                break;
            }
            self.collect_pipeline(&item.0.first)?;
            if self.execution_terminated {
                break;
            }
            let mut static_success = self.static_pipeline_success(&item.0.first)?;
            if self.positional_context.is_none() {
                self.collect_unscoped_and_or(&item.0.additional, &mut static_success)?;
                continue;
            }
            let (mut chain_success, mut chain_failure) =
                self.take_positional_outcomes(item.0.first.bang, static_success);
            for additional in &item.0.additional {
                if self.execution_terminated {
                    break;
                }
                let (execute_on_success, pipeline) = match additional {
                    AndOr::And(pipeline) => (true, pipeline),
                    AndOr::Or(pipeline) => (false, pipeline),
                };
                let (selected, skipped) = if execute_on_success {
                    (chain_success.take(), chain_failure.take())
                } else {
                    (chain_failure.take(), chain_success.take())
                };
                let definitely_executes = static_success == Some(execute_on_success);
                let (next_success, next_failure, pipeline_success) =
                    if let Some(selected) = selected {
                        self.positional_context = Some(selected);
                        self.with_positional_branch_execution(definitely_executes, |collector| {
                            collector.collect_pipeline(pipeline)
                        })?;
                        let pipeline_success = self.static_pipeline_success(pipeline)?;
                        let (success, failure) =
                            self.take_positional_outcomes(pipeline.bang, pipeline_success);
                        (success, failure, pipeline_success)
                    } else {
                        (None, None, None)
                    };
                if execute_on_success {
                    chain_success = next_success;
                    chain_failure =
                        super::shell_state::merge_positional_contexts(skipped, next_failure);
                    static_success = super::and_status(static_success, pipeline_success);
                } else {
                    chain_success =
                        super::shell_state::merge_positional_contexts(skipped, next_success);
                    chain_failure = next_failure;
                    static_success = super::or_status(static_success, pipeline_success);
                }
                self.positional_context = super::shell_state::merge_positional_contexts(
                    chain_success.clone(),
                    chain_failure.clone(),
                );
            }
            self.last_positional_status = match (chain_success.is_some(), chain_failure.is_some()) {
                (true, false) => Some(true),
                (false, true) => Some(false),
                _ => None,
            };
            self.last_positional_success = chain_success;
            self.last_positional_failure = chain_failure;
        }
        Ok(())
    }

    fn collect_unscoped_and_or(
        &mut self,
        additional: &[AndOr],
        static_success: &mut Option<bool>,
    ) -> Result<(), String> {
        for additional in additional {
            if self.execution_terminated {
                break;
            }
            match additional {
                AndOr::And(_) if *static_success == Some(false) => {}
                AndOr::Or(_) if *static_success == Some(true) => {}
                AndOr::And(pipeline) => {
                    let definitely_executes = *static_success == Some(true);
                    self.with_execution_certainty(definitely_executes, |collector| {
                        collector.collect_pipeline(pipeline)
                    })?;
                    *static_success =
                        super::and_status(*static_success, self.static_pipeline_success(pipeline)?);
                }
                AndOr::Or(pipeline) => {
                    let definitely_executes = *static_success == Some(false);
                    self.with_execution_certainty(definitely_executes, |collector| {
                        collector.collect_pipeline(pipeline)
                    })?;
                    *static_success =
                        super::or_status(*static_success, self.static_pipeline_success(pipeline)?);
                }
            }
        }
        Ok(())
    }

    pub(super) fn collect_case_clause(
        &mut self,
        command: &CaseClauseCommand,
    ) -> Result<(), String> {
        self.collect_word_commands(&command.value)?;
        let value = self.command_word(&command.value)?;
        let mut force_next_definite = false;
        let mut force_next_possible = false;
        let mut prior_possible_match = false;
        for case in &command.cases {
            for pattern in &case.patterns {
                self.collect_word_commands(pattern)?;
            }
            let mut match_state = if force_next_definite {
                Some(true)
            } else if force_next_possible {
                None
            } else {
                self.static_case_match(&value, &case.patterns)?
            };
            if prior_possible_match && match_state == Some(true) {
                match_state = None;
            }
            force_next_definite = false;
            force_next_possible = false;
            if match_state != Some(false) {
                if let Some(commands) = &case.cmd {
                    self.with_execution_certainty(
                        match_state == Some(true) && !prior_possible_match,
                        |collector| collector.collect_list(commands),
                    )?;
                }
                match case.post_action {
                    CaseItemPostAction::ExitCase if match_state == Some(true) => break,
                    CaseItemPostAction::UnconditionallyExecuteNextCaseItem => {
                        force_next_definite = match_state == Some(true);
                        force_next_possible = match_state.is_none();
                    }
                    CaseItemPostAction::ContinueEvaluatingCases | CaseItemPostAction::ExitCase => {}
                }
            }
            prior_possible_match |= match_state.is_none();
        }
        Ok(())
    }

    fn static_case_match(
        &self,
        value: &str,
        patterns: &[brush_parser::ast::Word],
    ) -> Result<Option<bool>, String> {
        if value == DYNAMIC_SHELL_WORD {
            return Ok(None);
        }
        let mut all_exact = true;
        for pattern in patterns {
            let pattern = self.command_word(pattern)?;
            if pattern == DYNAMIC_SHELL_WORD
                || pattern
                    .chars()
                    .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '(' | ')' | '|' | '\\'))
            {
                all_exact = false;
            } else if pattern == value || self.nocasematch && pattern.eq_ignore_ascii_case(value) {
                return Ok(Some(true));
            }
        }
        Ok(all_exact.then_some(false))
    }

    pub(super) fn collect_loop(
        &mut self,
        command: &WhileOrUntilClauseCommand,
        until: bool,
    ) -> Result<(), String> {
        self.collect_list(&command.0)?;
        let status = self.static_list_success(&command.0)?;
        let body_executes = status.map(|success| if until { !success } else { success });
        if body_executes != Some(false) {
            self.with_execution_certainty(body_executes == Some(true), |collector| {
                collector.collect_list(&command.1.list)
            })?;
        }
        Ok(())
    }

    pub(super) fn collect_if_clause(&mut self, command: &IfClauseCommand) -> Result<(), String> {
        self.collect_list(&command.condition)?;
        let status = self.static_list_success(&command.condition)?;
        if status != Some(false) {
            self.with_execution_certainty(status == Some(true), |collector| {
                collector.collect_list(&command.then)
            })?;
        }
        if status == Some(true) {
            return Ok(());
        }
        let mut branch_is_definite = status == Some(false);
        for branch in command.elses.iter().flatten() {
            let branch_status = if let Some(condition) = &branch.condition {
                self.with_execution_certainty(branch_is_definite, |collector| {
                    collector.collect_list(condition)
                })?;
                self.static_list_success(condition)?
            } else {
                Some(true)
            };
            if branch_status != Some(false) {
                self.with_execution_certainty(
                    branch_is_definite && branch_status == Some(true),
                    |collector| collector.collect_list(&branch.body),
                )?;
            }
            match branch_status {
                Some(true) => break,
                Some(false) => {}
                None => branch_is_definite = false,
            }
        }
        Ok(())
    }

    pub(super) fn static_list_success(&self, list: &CompoundList) -> Result<Option<bool>, String> {
        let Some(item) = list.0.last() else {
            return Ok(None);
        };
        let mut status = self.static_pipeline_success(&item.0.first)?;
        for additional in &item.0.additional {
            status = match additional {
                AndOr::And(pipeline) => {
                    super::and_status(status, self.static_pipeline_success(pipeline)?)
                }
                AndOr::Or(pipeline) => {
                    super::or_status(status, self.static_pipeline_success(pipeline)?)
                }
            };
        }
        Ok(status)
    }
}
