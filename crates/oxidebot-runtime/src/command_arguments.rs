//! Parsed command arguments indexed by schema name and stable field ID.

use super::*;

/// Parsed values indexed by both schema field name and deterministic field ID.
#[derive(Clone, Debug, Default)]
pub struct ParsedArguments {
    values: HashMap<Arc<str>, Vec<CommandValue>>,
    values_by_id: HashMap<CommandFieldId, Vec<CommandValue>>,
    present: HashSet<Arc<str>>,
    present_ids: HashSet<CommandFieldId>,
    flags: HashSet<Arc<str>>,
    flag_ids: HashSet<CommandFieldId>,
    counts: HashMap<Arc<str>, u32>,
    counts_by_id: HashMap<CommandFieldId, u32>,
}

impl ParsedArguments {
    /// Returns whether the named field was supplied.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.present.contains(name)
    }

    /// Returns whether the field with `id` was supplied.
    #[must_use]
    pub fn contains_id(&self, id: CommandFieldId) -> bool {
        self.present_ids.contains(&id)
    }

    /// Converts the first value of a required named field.
    pub fn required<T>(&self, name: &str) -> Result<T, CommandParseError>
    where
        T: FromCommandValue,
    {
        let value = self
            .values
            .get(name)
            .and_then(|values| values.first())
            .cloned()
            .ok_or_else(|| CommandParseError::MissingArgument {
                name: Arc::from(name),
                prompt: Arc::from(format!("请输入 {name}：")),
            })?;
        T::from_command_value(value)
    }

    /// Converts the first value of a required field identified by `id`.
    pub fn required_id<T>(&self, id: CommandFieldId) -> Result<T, CommandParseError>
    where
        T: FromCommandValue,
    {
        let value = self
            .values_by_id
            .get(&id)
            .and_then(|values| values.first())
            .cloned()
            .ok_or(CommandParseError::MissingFieldId { id })?;
        T::from_command_value(value)
    }

    /// Converts the first named field value when it is present.
    pub fn optional<T>(&self, name: &str) -> Result<Option<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values
            .get(name)
            .and_then(|values| values.first())
            .cloned()
            .map(T::from_command_value)
            .transpose()
    }

    /// Converts the first field value by stable ID when it is present.
    pub fn optional_id<T>(&self, id: CommandFieldId) -> Result<Option<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values_by_id
            .get(&id)
            .and_then(|values| values.first())
            .cloned()
            .map(T::from_command_value)
            .transpose()
    }

    /// Converts every value of a named field in input order.
    pub fn many<T>(&self, name: &str) -> Result<Vec<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values
            .get(name)
            .into_iter()
            .flatten()
            .cloned()
            .map(T::from_command_value)
            .collect()
    }

    /// Converts every value of a field by stable ID in input order.
    pub fn many_id<T>(&self, id: CommandFieldId) -> Result<Vec<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values_by_id
            .get(&id)
            .into_iter()
            .flatten()
            .cloned()
            .map(T::from_command_value)
            .collect()
    }

    /// Returns a named boolean flag's value.
    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    /// Returns a boolean flag's value by stable field ID.
    #[must_use]
    pub fn flag_id(&self, id: CommandFieldId) -> bool {
        self.flag_ids.contains(&id)
    }

    /// Returns the count action result for a named field.
    #[must_use]
    pub fn count(&self, name: &str) -> u32 {
        self.counts.get(name).copied().unwrap_or_default()
    }

    /// Returns the count action result by stable field ID.
    #[must_use]
    pub fn count_id(&self, id: CommandFieldId) -> u32 {
        self.counts_by_id.get(&id).copied().unwrap_or_default()
    }

    /// Borrows every lossless value of a named field.
    #[must_use]
    pub fn values(&self, name: &str) -> &[CommandValue] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }

    /// Borrows every lossless value of a field by stable ID.
    #[must_use]
    pub fn values_id(&self, id: CommandFieldId) -> &[CommandValue] {
        self.values_by_id.get(&id).map_or(&[], Vec::as_slice)
    }

    pub(in crate::command) fn mark_present(&mut self, spec: &ArgumentSpec) {
        self.present.insert(Arc::clone(&spec.name));
        self.present_ids.insert(spec.id);
    }

    pub(in crate::command) fn insert_value(&mut self, spec: &ArgumentSpec, value: CommandValue) {
        self.mark_present(spec);
        self.values
            .entry(Arc::clone(&spec.name))
            .or_default()
            .push(value.clone());
        self.values_by_id.entry(spec.id).or_default().push(value);
    }

    pub(in crate::command) fn set_flag(&mut self, spec: &ArgumentSpec, value: bool) {
        self.mark_present(spec);
        if value {
            self.flags.insert(Arc::clone(&spec.name));
            self.flag_ids.insert(spec.id);
        } else {
            self.flags.remove(&spec.name);
            self.flag_ids.remove(&spec.id);
        }
    }

    pub(in crate::command) fn increment_count(&mut self, spec: &ArgumentSpec) {
        self.mark_present(spec);
        *self.counts.entry(Arc::clone(&spec.name)).or_default() += 1;
        *self.counts_by_id.entry(spec.id).or_default() += 1;
    }

    pub(in crate::command) fn set_count(&mut self, spec: &ArgumentSpec, value: u32) {
        self.mark_present(spec);
        self.counts.insert(Arc::clone(&spec.name), value);
        self.counts_by_id.insert(spec.id, value);
    }

    pub(in crate::command) fn validate(
        &self,
        schema: &CommandSchema,
        locale: Option<&str>,
    ) -> Result<(), CommandParseError> {
        for spec in schema.arguments() {
            if spec.is_required() && !self.contains(spec.name()) {
                return Err(CommandParseError::MissingArgument {
                    name: Arc::clone(&spec.name),
                    prompt: Arc::from(spec.prompt_text_for(locale)),
                });
            }
            if !self.contains(spec.name()) {
                continue;
            }
            for required in &spec.requires {
                if !self.contains(required) {
                    return Err(CommandParseError::Requires {
                        argument: Arc::clone(&spec.name),
                        required: Arc::clone(required),
                    });
                }
            }
            for conflict in &spec.conflicts {
                if self.contains(conflict) {
                    return Err(CommandParseError::Conflict {
                        left: Arc::clone(&spec.name),
                        right: Arc::clone(conflict),
                    });
                }
            }
            for value in self.values(spec.name()) {
                validate_argument_value(spec, value)?;
            }
        }
        for group in schema.groups() {
            let present = group
                .arguments_ref()
                .iter()
                .filter(|name| self.contains(name))
                .cloned()
                .collect::<Vec<_>>();
            if group.required && present.is_empty() {
                return Err(CommandParseError::MissingArgumentGroup {
                    group: Arc::clone(&group.name),
                    members: group.arguments_ref().to_vec().into(),
                });
            }
            if !group.multiple && present.len() > 1 {
                return Err(CommandParseError::ArgumentGroupConflict {
                    group: Arc::clone(&group.name),
                    members: present.into(),
                });
            }
        }
        Ok(())
    }
}
