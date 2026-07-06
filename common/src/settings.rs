use arc_swap::ArcSwap;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
// ── Key ─────────────────────────────────────────────────────────

/// A field's identity and its Rust read type.
pub struct SettingKey<T: SettingType + 'static> {
    name: &'static str,
    _pd: PhantomData<fn() -> T>,
}
impl<T: SettingType + 'static> SettingKey<T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _pd: PhantomData,
        }
    }
    pub fn as_str(&self) -> &'static str {
        self.name
    }
    pub fn env_name(&self) -> String {
        self.as_str().to_ascii_uppercase()
    }
}
impl<T: SettingType + 'static> Clone for SettingKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: SettingType + 'static> Copy for SettingKey<T> {}

// ── Kind ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    U16,
    U64,
    I64,
    F64,
    Bool,
    String,
    StringList,
    Enum(&'static [&'static str]),
}
impl SettingKind {
    pub fn name(&self) -> &'static str {
        match self {
            SettingKind::U16 => "u16",
            SettingKind::U64 => "u64",
            SettingKind::I64 => "i64",
            SettingKind::F64 => "f64",
            SettingKind::Bool => "bool",
            SettingKind::String => "string",
            SettingKind::StringList => "string_list",
            SettingKind::Enum(_) => "enum",
        }
    }
    pub fn variants(&self) -> Option<Vec<String>> {
        match self {
            SettingKind::Enum(variants) => Some(variants.iter().map(|s| s.to_string()).collect()),
            _ => None,
        }
    }
}

/// Marker trait for nullable types (only Option<T> implements this)
pub trait IsNullable: SettingType {}

/// Maps a field's Rust read type to its `SettingKind`
pub trait SettingType: Sized + Serialize {
    const KIND: SettingKind;
    fn from_snapshot(snap: &SettingsSnapshot, key: &str) -> Self;
}
impl SettingType for bool {
    const KIND: SettingKind = SettingKind::Bool;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k).and_then(|v| v.as_bool()).unwrap_or(false)
    }
}
impl SettingType for u16 {
    const KIND: SettingKind = SettingKind::U16;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k).and_then(|v| v.as_u64()).unwrap_or(0) as u16
    }
}
impl SettingType for u64 {
    const KIND: SettingKind = SettingKind::U64;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k).and_then(|v| v.as_u64()).unwrap_or(0)
    }
}
impl SettingType for usize {
    const KIND: SettingKind = SettingKind::U64;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k).and_then(|v| v.as_u64()).unwrap_or(0) as usize
    }
}
impl SettingType for i64 {
    const KIND: SettingKind = SettingKind::I64;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k).and_then(|v| v.as_i64()).unwrap_or(0)
    }
}
impl SettingType for f64 {
    const KIND: SettingKind = SettingKind::F64;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }
}
impl SettingType for String {
    const KIND: SettingKind = SettingKind::String;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }
}
impl SettingType for Option<String> {
    const KIND: SettingKind = SettingKind::String;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        match s.v(k).and_then(|v| v.as_str()) {
            Some(x) if !x.is_empty() => Some(x.to_string()),
            _ => None,
        }
    }
}
impl IsNullable for Option<String> {}
impl SettingType for Vec<String> {
    const KIND: SettingKind = SettingKind::StringList;
    fn from_snapshot(s: &SettingsSnapshot, k: &str) -> Self {
        s.v(k)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}
#[macro_export]
macro_rules! wire_enum {
    ($t:ty) => {
        impl $crate::settings::SettingType for $t {
            const KIND: $crate::settings::SettingKind =
                $crate::settings::SettingKind::Enum(<$t as strum::VariantNames>::VARIANTS);
            fn from_snapshot(s: &$crate::settings::SettingsSnapshot, key: &str) -> Self {
                s.v(key)
                    .and_then(|v| v.as_str())
                    .and_then(|s| <$t as ::std::str::FromStr>::from_str(s).ok())
                    .unwrap_or_else(|| <$t as strum::VariantNames>::VARIANTS[0].parse().unwrap())
            }
        }
    };
}

// ── Spec ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum DefaultValue {
    None,
    Static(Value),
    Computed(fn(&SettingsSnapshot) -> Value, &'static str),
}
impl Serialize for DefaultValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            DefaultValue::None => serializer.serialize_none(),
            DefaultValue::Static(v) => v.serialize(serializer),
            DefaultValue::Computed(_, description) => serializer.serialize_str(description),
        }
    }
}

#[derive(Clone)]
pub struct SettingSpec {
    pub key: &'static str,
    pub env: String,
    pub kind: SettingKind,
    pub group: &'static str,
    pub default: DefaultValue,
    /// Secrets are redacted and are only editable by envs
    pub secret: bool,
    /// Can be edited at runtime
    pub runtime_editable: bool,
    /// May be editable at runtime, but requires a restart to take effect
    pub restart_required: bool,
    pub routine: Option<&'static str>,
    pub description: &'static str,
    pub example: &'static str,
    /// If true, missing values default to None (only for Option<T> types)
    pub nullable: bool,
}

impl SettingSpec {
    pub fn new<T: SettingType + 'static>(key: SettingKey<T>, group: &'static str) -> Self {
        Self {
            key: key.as_str(),
            env: key.env_name(),
            kind: T::KIND,
            group,
            default: DefaultValue::None,
            secret: false,
            runtime_editable: true,
            restart_required: false,
            routine: None,
            description: "",
            example: "",
            nullable: false,
        }
    }
    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }
    pub fn default(mut self, d: &'static str) -> Self {
        let v = coerce_str(self.kind, d)
            .unwrap_or_else(|e| panic!("Invalid default for {}: {}", self.key, e));
        self.default = DefaultValue::Static(v);
        self
    }
    pub fn default_computed(
        mut self,
        f: fn(&SettingsSnapshot) -> Value,
        description: &'static str,
    ) -> Self {
        self.default = DefaultValue::Computed(f, description);
        self
    }
    pub fn core(mut self) -> Self {
        self.runtime_editable = false;
        self
    }
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self.runtime_editable = false;
        self
    }
    pub fn restart_required(mut self) -> Self {
        self.restart_required = true;
        self
    }
    pub fn routine(mut self, routine: &'static str) -> Self {
        self.routine = Some(routine);
        self
    }
    pub fn doc(mut self, description: &'static str, example: &'static str) -> Self {
        self.description = description;
        self.example = example;
        self
    }
}

// ── Provenance & errors ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Default,
    Env,
    Db,
}

#[derive(Debug)]
pub enum SettingsError {
    UnknownKey(String),
    NotEditable(String),
    Locked(String),
    Invalid { key: String, msg: String },
    MissingRequired(Vec<String>),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::UnknownKey(k) => write!(f, "unknown setting '{k}'"),
            SettingsError::NotEditable(k) => {
                write!(
                    f,
                    "setting '{k}' is a core field and can only be set via its environment variable"
                )
            }
            SettingsError::Locked(k) => {
                write!(
                    f,
                    "setting '{k}' is defined by an environment variable and cannot be changed at runtime"
                )
            }
            SettingsError::Invalid { key, msg } => write!(f, "invalid value for '{key}': {msg}"),
            SettingsError::MissingRequired(vars) => {
                write!(
                    f,
                    "missing required environment variable(s): {}",
                    vars.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for SettingsError {}

// ── Snapshot ─────────────────────────────────────────────────────────────────────

/// A merged, immutable view of every field at a point in time.
#[derive(Debug)]
pub struct SettingsSnapshot {
    values: HashMap<&'static str, Value>,
    sources: HashMap<&'static str, Source>,
}

impl SettingsSnapshot {
    /// Raw JSON value for a key (used by [`SettingType`] impls, incl. `wire_enum!`-generated ones).
    pub fn v(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    /// Read a field by its typed key; the return type is fixed by the key (see [`SettingKey`]).
    pub fn get<T: SettingType>(&self, key: SettingKey<T>) -> T {
        T::from_snapshot(self, key.as_str())
    }
    pub fn source<T: SettingType>(&self, key: SettingKey<T>) -> Source {
        self.sources
            .get(key.as_str())
            .copied()
            .unwrap_or(Source::Default)
    }
}

// ── Wire model ───────────────────────────────────────────────────────────────────

/// One field as sent to the dashboard. Secret values are redacted (only `is_set` is exposed).
#[derive(Clone, Serialize)]
pub struct FieldMeta {
    pub key: String,
    pub env: String,
    pub group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub is_set: bool,
    pub default_value: DefaultValue,
    pub source: Source,
    /// `source == Env` — rendered read-only with a "defined by environment" badge.
    pub locked: bool,
    /// `false` = core/env-only field (secrets, topology).
    pub runtime_editable: bool,
    pub restart_required: bool,
    pub secret: bool,
    /// The field may be empty/`None` (an `Option<T>` setting).
    pub nullable: bool,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routine: Option<String>,
    pub description: String,
    pub example: String,
}

// ── Settings ─────────────────────────────────────────────────────────────────────

/// The live, hot-swappable configuration held in `AppState`.
pub struct Settings {
    specs: Vec<SettingSpec>,
    current: ArcSwap<SettingsSnapshot>,
}

impl Settings {
    /// Merge `default → env → db_overrides` into the initial snapshot.
    pub fn load(
        registry: &[SettingSpec],
        db_overrides: &HashMap<String, Value>,
    ) -> Result<Self, SettingsError> {
        Self::load_with_env(registry, db_overrides, &std_env)
    }
    pub fn load_with_env(
        specs: &[SettingSpec],
        db_overrides: &HashMap<String, Value>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, SettingsError> {
        let snap = build_snapshot(&specs, db_overrides, env)?;
        Ok(Self {
            specs: Vec::from(specs),
            current: ArcSwap::from_pointee(snap),
        })
    }

    /// Current merged snapshot (cheap `Arc` clone).
    pub fn snapshot(&self) -> Arc<SettingsSnapshot> {
        self.current.load_full()
    }

    /// Rebuild from the given DB overrides and atomically publish (after a PATCH/DELETE).
    pub fn reload(&self, db_overrides: &HashMap<String, Value>) -> Result<(), SettingsError> {
        let snap = build_snapshot(&self.specs, db_overrides, &std_env)?;
        self.current.store(Arc::new(snap));
        Ok(())
    }

    /// A new `Settings` with the same fields as this one but the given env-name overrides applied
    /// (the current snapshot's values seed the env layer). Handy for tests and for spinning a variant
    /// config without re-reading the process environment.
    pub fn cloned_with(&self, env_overrides: &[(&str, String)]) -> Self {
        let snap = self.current.load();
        let mut env: HashMap<String, String> = HashMap::new();
        for spec in &self.specs {
            if let Some(v) = snap.values.get(spec.key) {
                env.insert(spec.env.clone(), value_to_env_string(v));
            }
        }
        for (k, val) in env_overrides {
            env.insert(k.to_string(), val.clone());
        }
        let lookup = move |k: &str| env.get(k).cloned();
        let new_snap = build_snapshot(&self.specs, &HashMap::new(), &lookup)
            .expect("cloned_with: current values must re-parse");
        Settings {
            specs: self.specs.clone(),
            current: ArcSwap::from_pointee(new_snap),
        }
    }

    fn spec(&self, key: &str) -> Option<&SettingSpec> {
        self.specs.iter().find(|s| s.key == key)
    }

    pub fn is_locked<T: SettingType>(&self, k: SettingKey<T>) -> bool {
        self.is_locked_str(k.as_str())
    }

    /// A setting is locked if defined by an env variable.
    pub fn is_locked_str(&self, key: &str) -> bool {
        self.current.load().sources.get(key).copied() == Some(Source::Env)
    }

    /// Validate a proposed override: reject unknown / core / env-locked / mistyped values, returning
    /// the coerced JSON ready to persist as a DB override.
    pub fn validate_override<T: SettingType>(
        &self,
        k: SettingKey<T>,
        value: &Value,
    ) -> Result<Value, SettingsError> {
        self.validate_override_str(k.as_str(), value)
    }

    pub fn validate_override_str(&self, key: &str, value: &Value) -> Result<Value, SettingsError> {
        let spec = self
            .spec(key)
            .ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        if !spec.runtime_editable {
            return Err(SettingsError::NotEditable(key.to_string()));
        }
        if self.is_locked_str(key) {
            return Err(SettingsError::Locked(key.to_string()));
        }
        coerce_value(spec.kind, value).map_err(|msg| SettingsError::Invalid {
            key: key.to_string(),
            msg,
        })
    }

    /// Full field metadata for the dashboard
    pub fn field_meta(&self) -> Vec<FieldMeta> {
        let snap = self.current.load();
        self.specs
            .iter()
            .map(|s| {
                let is_set = snap.values.contains_key(s.key);
                let source = snap.sources.get(s.key).copied().unwrap_or(Source::Default);
                let value = if s.secret {
                    None
                } else {
                    snap.values.get(s.key).cloned()
                };
                FieldMeta {
                    key: s.key.to_string(),
                    env: s.env.clone(),
                    group: s.group.to_string(),
                    value,
                    is_set,
                    default_value: s.default.clone(),
                    source,
                    locked: source == Source::Env,
                    runtime_editable: s.runtime_editable,
                    restart_required: s.restart_required,
                    secret: s.secret,
                    nullable: s.nullable,
                    kind: s.kind.name().to_string(),
                    variants: s.kind.variants(),
                    routine: s.routine.map(str::to_string),
                    description: s.description.to_string(),
                    example: s.example.to_string(),
                }
            })
            .collect()
    }

    pub fn get<T: SettingType>(&self, k: SettingKey<T>) -> T {
        T::from_snapshot(&self.current.load(), k.as_str())
    }
}

fn std_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Render a snapshot value back to its env-string form (for [`Settings::cloned_with`]).
fn value_to_env_string(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(),
    }
}

fn build_snapshot(
    specs: &[SettingSpec],
    db_overrides: &HashMap<String, Value>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<SettingsSnapshot, SettingsError> {
    let mut values: HashMap<&'static str, Value> = HashMap::new();
    let mut sources: HashMap<&'static str, Source> = HashMap::new();

    // First pass: env, db overrides, and static defaults
    for spec in specs {
        // env LOCKS; else db override; else static default; else absent.
        if let Some(raw) = env(&spec.env) {
            if let Ok(v) = coerce_str(spec.kind, &raw) {
                values.insert(spec.key, v);
                sources.insert(spec.key, Source::Env);
                continue;
            }
        }
        if spec.runtime_editable {
            if let Some(dbv) = db_overrides.get(spec.key) {
                values.insert(spec.key, dbv.clone());
                sources.insert(spec.key, Source::Db);
                continue;
            }
        }
        if let DefaultValue::Static(d) = &spec.default {
            values.insert(spec.key, d.clone());
            sources.insert(spec.key, Source::Default);
        }
        // else: absent (no entry)
    }

    // Create temporary snapshot for computed defaults evaluation
    let snap = SettingsSnapshot {
        values: values.clone(),
        sources: sources.clone(),
    };

    // Validate (conditional) requiredness against the assembled snapshot, without computed defaults.
    let missing: Vec<String> = specs
        .iter()
        .filter(|s| {
            !snap.values.contains_key(s.key)
                && !matches!(s.default, DefaultValue::Computed(_, _))
                && !s.nullable
        })
        .map(|s| s.env.clone())
        .collect();
    if !missing.is_empty() {
        return Err(SettingsError::MissingRequired(missing));
    }

    // Second pass: computed defaults (fills in missing values that weren't set above)
    for spec in specs {
        if !values.contains_key(spec.key) {
            if let DefaultValue::Computed(f, _) = &spec.default {
                let v = f(&snap);
                values.insert(spec.key, v);
                sources.insert(spec.key, Source::Default);
            }
        }
    }

    // Third pass: set None (Value::Null) for missing nullable settings
    for spec in specs {
        if !values.contains_key(spec.key) && spec.nullable {
            values.insert(spec.key, Value::Null);
            sources.insert(spec.key, Source::Default);
        }
    }

    let snap = SettingsSnapshot { values, sources };

    // Validate (conditional) requiredness against the assembled snapshot — only for non-nullable settings.
    let missing: Vec<String> = specs
        .iter()
        .filter(|s| !snap.values.contains_key(s.key))
        .map(|s| s.env.clone())
        .collect();
    if !missing.is_empty() {
        return Err(SettingsError::MissingRequired(missing));
    }
    Ok(snap)
}

/// Parse an env/default string into the JSON value for `kind`.
fn coerce_str(kind: SettingKind, s: &str) -> Result<Value, String> {
    let s = s.trim();
    match kind {
        SettingKind::U16 => s
            .parse::<u16>()
            .map(|n| Value::from(n as u64))
            .map_err(|_| "expected a port (0-65535)".into()),
        SettingKind::U64 => s
            .parse::<u64>()
            .map(Value::from)
            .map_err(|_| "expected a non-negative integer".into()),
        SettingKind::I64 => s
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| "expected an integer".into()),
        SettingKind::F64 => s
            .parse::<f64>()
            .map(Value::from)
            .map_err(|_| "expected a number".into()),
        SettingKind::Bool => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(Value::Bool(true)),
            "false" | "0" | "no" => Ok(Value::Bool(false)),
            _ => Err("expected a boolean (true/false)".into()),
        },
        SettingKind::String => Ok(Value::String(s.to_string())),
        SettingKind::StringList => Ok(Value::Array(
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| Value::String(p.to_string()))
                .collect(),
        )),
        SettingKind::Enum(variants) => {
            if variants.contains(&s) {
                Ok(Value::String(s.to_string()))
            } else {
                Err(format!("expected one of: {}", variants.join(", ")))
            }
        }
    }
}

/// Validate/normalise a JSON value from a PATCH into the canonical value for `kind`.
fn coerce_value(kind: SettingKind, v: &Value) -> Result<Value, String> {
    match kind {
        SettingKind::U16 => match v {
            Value::Number(n) if n.as_u64().is_some_and(|x| x <= u16::MAX as u64) => {
                Ok(Value::from(n.as_u64().unwrap()))
            }
            Value::String(s) => coerce_str(kind, s),
            _ => Err("expected a port (0-65535)".into()),
        },
        SettingKind::U64 => match v {
            Value::Number(n) if n.as_u64().is_some() => Ok(Value::from(n.as_u64().unwrap())),
            Value::String(s) => coerce_str(kind, s),
            _ => Err("expected a non-negative integer".into()),
        },
        SettingKind::I64 => match v {
            Value::Number(n) if n.as_i64().is_some() => Ok(Value::from(n.as_i64().unwrap())),
            Value::String(s) => coerce_str(kind, s),
            _ => Err("expected an integer".into()),
        },
        SettingKind::F64 => match v {
            Value::Number(n) if n.as_f64().is_some() => Ok(Value::from(n.as_f64().unwrap())),
            Value::String(s) => coerce_str(kind, s),
            _ => Err("expected a number".into()),
        },
        SettingKind::Bool => match v {
            Value::Bool(b) => Ok(Value::Bool(*b)),
            Value::String(s) => coerce_str(kind, s),
            _ => Err("expected a boolean".into()),
        },
        SettingKind::String => match v {
            Value::String(s) => Ok(Value::String(s.clone())),
            _ => Err("expected a string".into()),
        },
        SettingKind::StringList => match v {
            Value::Array(arr) => {
                let mut out = Vec::new();
                for item in arr {
                    match item {
                        Value::String(s) => out.push(Value::String(s.clone())),
                        _ => return Err("expected an array of strings".into()),
                    }
                }
                Ok(Value::Array(out))
            }
            Value::String(s) => coerce_str(kind, s),
            _ => Err("expected an array of strings".into()),
        },
        SettingKind::Enum(_) => match v {
            Value::String(s) => coerce_str(kind, s),
            _ => Err("expected a string".into()),
        },
    }
}
