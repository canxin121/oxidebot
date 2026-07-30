//! Native command publication and user-facing command-tree views.

use super::*;

impl Command {
    /// Converts this command grammar into a portable native command definition.
    #[must_use]
    pub fn definition(&self) -> CommandDefinition {
        let mut options = self
            .global_schema
            .as_ref()
            .map_or_else(Vec::new, CommandSchema::native_options);
        if let Some(schema) = &self.schema {
            options.extend(schema.native_options());
        }
        options.extend(self.branches.iter().map(CommandBranch::native_option));
        CommandDefinition {
            id: Some(format!("oxidebot:{:016x}", self.id.0)),
            name: Localized::new(normalize_native_name(&self.name)),
            description: if self.description.is_empty() {
                Localized::new(self.name.to_string())
            } else {
                self.description.to_core()
            },
            kind: CommandKind::ChatInput,
            options,
            default_permissions: None,
            contexts: std::iter::once(CommandContext::Any).collect(),
            ephemeral: false,
            platform_data: None,
        }
    }

    /// Suggests branch, option, argument, and choice completions for input.
    #[must_use]
    pub fn suggest(&self, input: &str, cursor: usize, locale: Option<&str>) -> Vec<CompletionItem> {
        suggest_for_command(self, input, cursor, locale)
    }

    /// Builds usage text for a selected command-tree branch path.
    #[must_use]
    pub fn usage_for(&self, branch_names: &[Arc<str>]) -> String {
        let mut usage = self.display_name();
        for branch in branch_names {
            usage.push(' ');
            usage.push_str(branch);
        }
        if !branch_children(self, branch_names).is_empty() {
            usage.push_str(" <subcommand>");
        }
        let schema = self.schema_for_branch_names(branch_names);
        for argument in schema.arguments() {
            if argument.is_hidden() {
                continue;
            }
            usage.push(' ');
            usage.push_str(&argument.usage_fragment());
        }
        usage
    }

    pub(in crate::command) fn schema_for_branch_names(
        &self,
        branch_names: &[Arc<str>],
    ) -> CommandSchema {
        let mut schema = self.global_schema.clone().unwrap_or_default();
        if let Some(root_schema) = &self.schema {
            schema = schema.merged(root_schema);
        }
        let mut children = self.branches.as_slice();
        for name in branch_names {
            let Some(branch) = children.iter().find(|branch| branch.name == *name) else {
                break;
            };
            if let Some(branch_schema) = &branch.schema {
                schema = schema.merged(branch_schema);
            }
            children = branch.children.as_slice();
        }
        schema
    }
}
