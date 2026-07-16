use std::collections::HashMap;

use brush_parser::ast::{
    CommandPrefixOrSuffixItem, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect, SimpleCommand,
};

use super::{CommandCollector, DYNAMIC_SHELL_WORD};

impl CommandCollector {
    /// Select the static fd-0 payload after applying Bash redirections left-to-right.
    pub(super) fn effective_stdin_payload(
        &self,
        command: &SimpleCommand,
    ) -> Result<Option<String>, String> {
        let mut payloads = HashMap::<i32, Option<String>>::new();
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
                        payloads.insert(fd.unwrap_or(0), Some(here_doc.doc.value.clone()));
                    }
                    IoRedirect::HereString(fd, word) => {
                        let value = self.command_word(word)?;
                        payloads.insert(
                            fd.unwrap_or(0),
                            (value != DYNAMIC_SHELL_WORD).then_some(value),
                        );
                    }
                    IoRedirect::File(fd, kind, target) => {
                        let target_fd = fd.unwrap_or_else(|| default_redirect_fd(kind));
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
        Ok(payloads.remove(&0).flatten())
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
