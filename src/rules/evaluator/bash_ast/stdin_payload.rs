use std::collections::HashMap;

use brush_parser::ast::{
    CommandPrefixOrSuffixItem, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect, SimpleCommand,
    Word,
};

use super::function_args::{expand_shell_here_string, expand_shell_heredoc};
use super::static_words::static_word_pieces;
use super::{CommandCollector, PositionalContext, DYNAMIC_SHELL_WORD};

pub(super) enum EffectiveStdin {
    Untouched,
    Replaced(Option<String>),
}

impl CommandCollector {
    fn here_string_word(&self, word: &Word) -> Result<String, String> {
        let source = self.positional_context.as_ref().map_or_else(
            || Ok(word.value.clone()),
            |context| {
                expand_shell_here_string(
                    &word.value,
                    &self.options,
                    context.zero_argument.as_deref(),
                    &context.arguments,
                )
            },
        )?;
        let pieces = brush_parser::word::parse(&source, &self.options)
            .map_err(|error| format!("Bash here-string word parse error: {error}"))?;
        Ok(static_word_pieces(&pieces).unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()))
    }

    pub(super) fn expand_positional_heredoc(&self, source: &str) -> Result<String, String> {
        let (zero_argument, arguments) =
            self.positional_context
                .as_ref()
                .map_or((None, &[][..]), |context| {
                    (
                        context.zero_argument.as_deref(),
                        context.arguments.as_slice(),
                    )
                });
        expand_shell_heredoc(source, &self.options, zero_argument, arguments)
    }

    pub(super) fn collect_source_stdin_payload(
        &mut self,
        command: &SimpleCommand,
        source_arguments: &[String],
    ) -> Result<(), String> {
        let payload = match self.effective_stdin_payload(command)? {
            EffectiveStdin::Replaced(payload) => payload,
            EffectiveStdin::Untouched => self.inherited_stdin.clone(),
        };
        let Some(payload) = payload else {
            return Ok(());
        };
        if source_arguments.is_empty() {
            return self.collect_source(&payload);
        }
        let zero_argument = self
            .positional_context
            .as_ref()
            .and_then(|context| context.zero_argument.clone());
        let saved_context = self.positional_context.replace(PositionalContext {
            zero_argument,
            arguments: source_arguments.to_vec(),
            possible_arguments: Vec::new(),
        });
        let saved_set_generation = self.positional_set_generation;
        let result = self.collect_source(&payload);
        let sourced_context = self.positional_context.take();
        if result.is_ok() && self.positional_set_generation != saved_set_generation {
            self.positional_context = sourced_context;
        } else {
            self.positional_context = saved_context;
            self.positional_set_generation = saved_set_generation;
        }
        result?;
        let success = self.last_positional_success.is_some();
        let failure = self.last_positional_failure.is_some();
        self.last_positional_success = success.then(|| self.positional_context.clone()).flatten();
        self.last_positional_failure = failure.then(|| self.positional_context.clone()).flatten();
        Ok(())
    }

    /// Select the static fd-0 payload after applying Bash redirections left-to-right.
    pub(super) fn effective_stdin_payload(
        &self,
        command: &SimpleCommand,
    ) -> Result<EffectiveStdin, String> {
        let mut payloads = HashMap::<i32, Option<String>>::new();
        if let Some(payload) = &self.inherited_stdin {
            payloads.insert(0, Some(payload.clone()));
        }
        let mut stdin_replaced = false;
        for items in [
            command.prefix.as_ref().map(|prefix| &prefix.0),
            command.suffix.as_ref().map(|suffix| &suffix.0),
        ]
        .into_iter()
        .flatten()
        {
            for item in items {
                let CommandPrefixOrSuffixItem::IoRedirect(redirect) = item else {
                    continue;
                };
                match redirect {
                    IoRedirect::HereDocument(fd, here_doc) => {
                        let target_fd = fd.unwrap_or(0);
                        stdin_replaced |= target_fd == 0;
                        let payload = if here_doc.requires_expansion {
                            self.expand_positional_heredoc(&here_doc.doc.value)?
                        } else {
                            here_doc.doc.value.clone()
                        };
                        payloads.insert(target_fd, Some(payload));
                    }
                    IoRedirect::HereString(fd, word) => {
                        let value = self.here_string_word(word)?;
                        let target_fd = fd.unwrap_or(0);
                        stdin_replaced |= target_fd == 0;
                        payloads.insert(target_fd, (value != DYNAMIC_SHELL_WORD).then_some(value));
                    }
                    IoRedirect::File(fd, kind, target) => {
                        let target_fd = fd.unwrap_or_else(|| default_redirect_fd(kind));
                        stdin_replaced |= target_fd == 0;
                        let payload = if matches!(kind, IoFileRedirectKind::DuplicateInput) {
                            duplicate_input_fd(target, self)?
                                .and_then(|source_fd| payloads.get(&source_fd).cloned().flatten())
                        } else {
                            None
                        };
                        payloads.insert(target_fd, payload);
                    }
                    IoRedirect::OutputAndError(_, _) => {}
                }
            }
        }
        Ok(match (stdin_replaced, payloads.remove(&0)) {
            (false, _) => EffectiveStdin::Untouched,
            (true, payload) => EffectiveStdin::Replaced(payload.flatten()),
        })
    }
}

fn default_redirect_fd(kind: &IoFileRedirectKind) -> i32 {
    match kind {
        IoFileRedirectKind::Read
        | IoFileRedirectKind::ReadAndWrite
        | IoFileRedirectKind::DuplicateInput => 0,
        IoFileRedirectKind::Write
        | IoFileRedirectKind::Append
        | IoFileRedirectKind::Clobber
        | IoFileRedirectKind::DuplicateOutput => 1,
    }
}

fn duplicate_input_fd(
    target: &IoFileRedirectTarget,
    collector: &CommandCollector,
) -> Result<Option<i32>, String> {
    match target {
        IoFileRedirectTarget::Fd(fd) => Ok(Some(*fd)),
        IoFileRedirectTarget::Duplicate(word) => {
            Ok(collector.command_word(word)?.parse::<i32>().ok())
        }
        IoFileRedirectTarget::Filename(_) | IoFileRedirectTarget::ProcessSubstitution(_, _) => {
            Ok(None)
        }
    }
}
