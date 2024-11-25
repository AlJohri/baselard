use std::{collections::HashMap, sync::Arc};

use serde::{de::VariantAccess, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};
use std::hash::{Hash, Hasher};

use crate::dag::DAGError;

/// Runtime values that flow through the DAG
#[derive(Debug, Clone)]
pub enum Data {
    Null,
    Integer(i32),
    Float(f64),
    Text(String),
    List(Vec<Data>),
    Json(Value),
    /// A channel for single-consumer asynchronous results, wrapped in an `Arc<Mutex>` for safe sharing.
    OneConsumerChannel(Arc<Mutex<Option<oneshot::Receiver<Data>>>>),
}

impl Hash for Data {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Data::Null => {
                "Null".hash(state);
            }
            Data::Integer(value) => {
                "Integer".hash(state);
                value.hash(state);
            }
            Data::Float(value) => {
                "Float".hash(state);
                value.to_bits().hash(state);
            }
            Data::Text(value) => {
                "Text".hash(state);
                value.hash(state);
            }
            Data::List(values) => {
                "List".hash(state);
                for value in values {
                    value.hash(state);
                }
            }
            Data::Json(value) => {
                "Json".hash(state);
                value.to_string().hash(state);
            }
            Data::OneConsumerChannel(_) => {
                // Treat channels as opaque and hash a constant value instead.
                "OneConsumerChannel".hash(state);
            }
        }
    }
}

impl PartialEq for Data {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Data::Null, Data::Null) => true,
            (Data::Integer(a), Data::Integer(b)) => a == b,
            (Data::Float(a), Data::Float(b)) => a == b,
            (Data::Text(a), Data::Text(b)) => a == b,
            (Data::List(a), Data::List(b)) => a == b,
            (Data::Json(a), Data::Json(b)) => a == b,
            // For channels, we'll consider them equal if they're both None
            (Data::OneConsumerChannel(a), Data::OneConsumerChannel(b)) => {
                // Compare if both are None
                matches!(
                    (a.try_lock().ok().as_ref().map(|g| g.is_none()),
                     b.try_lock().ok().as_ref().map(|g| g.is_none())),
                    (Some(true), Some(true))
                )
            },
            _ => false,
        }
    }
}

impl Serialize for Data {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Data::Null => serializer.serialize_unit_variant("Data", 0, "Null"),
            Data::Integer(i) => serializer.serialize_newtype_variant("Data", 1, "Integer", i),
            Data::Float(f) => serializer.serialize_newtype_variant("Data", 2, "Float", f),
            Data::Text(s) => serializer.serialize_newtype_variant("Data", 2, "Text", s),
            Data::List(list) => serializer.serialize_newtype_variant("Data", 3, "List", list),
            Data::Json(value) => serializer.serialize_newtype_variant("Data", 4, "Json", value),
            Data::OneConsumerChannel(_) => {
                // Serialize as an opaque placeholder
                serializer.serialize_unit_variant("Data", 5, "OneConsumerChannel")
            }
        }
    }
}

impl<'de> Deserialize<'de> for Data {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Null,
            Integer,
            Float,
            Text,
            List,
            Json,
            OneConsumerChannel,
        }

        struct DataVisitor;

        impl<'de> serde::de::Visitor<'de> for DataVisitor {
            type Value = Data;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an enum representing Data")
            }

            fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::EnumAccess<'de>,
            {
                match data.variant()? {
                    (Field::Null, _) => Ok(Data::Null),
                    (Field::Integer, variant) => variant.newtype_variant().map(Data::Integer),
                    (Field::Float, variant) => variant.newtype_variant().map(Data::Float),
                    (Field::Text, variant) => variant.newtype_variant().map(Data::Text),
                    (Field::List, variant) => variant.newtype_variant().map(Data::List),
                    (Field::Json, variant) => variant.newtype_variant().map(Data::Json),
                    (Field::OneConsumerChannel, _) => {
                        // Deserialize as opaque placeholder
                        Ok(Data::OneConsumerChannel(Arc::new(Mutex::new(None))))
                    }
                }
            }
        }

        deserializer.deserialize_enum(
            "Data",
            &["Null", "Integer", "Float", "Text", "List", "Json", "OneConsumerChannel"],
            DataVisitor,
        )
    }
}

/// Type information for validation during DAG construction
#[derive(Debug, Clone, PartialEq)]
pub enum DataType {
    /// Represents the absence of input for a component.
    Null,
    Integer,
    Float,
    Text,
    List(Box<DataType>),
    Json,
    Union(Vec<DataType>),
    /// Represents a single-consumer channel carrying a specific data type.
    OneConsumerChannel(Box<DataType>),
}

impl DataType {
    /// Determines whether one `DataType` is compatible with another.
    ///
    /// This function checks if a value of the current `DataType` (`self`) can be
    /// safely used as input where the target `DataType` (`other`) is expected.
    /// It supports direct type equivalence, union compatibility, and nested list type compatibility.
    ///
    /// ### Compatibility Rules:
    /// - **Exact Match**: Two data types are directly compatible if they are equal.
    /// - **Union Compatibility**: A `DataType` is compatible with a `DataType::Union` if it is compatible
    ///   with at least one of the types in the union.
    /// - **List Compatibility**: Two `DataType::List` types are compatible if their element types are compatible.
    /// - **Otherwise**: The types are considered incompatible.
    ///
    /// ### Parameters:
    /// - `other`: The target `DataType` to check compatibility against.
    ///
    /// ### Returns:
    /// - `true` if `self` is compatible with `other`.
    /// - `false` otherwise.
    ///
    /// ### Examples:
    /// #### Example 1: Direct Compatibility
    /// ```rust
    /// use baselard::component::DataType;
    /// let a = DataType::Integer;
    /// let b = DataType::Integer;
    /// assert!(a.is_compatible_with(&b)); // true
    /// ```
    ///
    /// #### Example 2: Union Compatibility
    /// ```rust
    /// use baselard::component::DataType;
    /// let source = DataType::Text;
    /// let target = DataType::Union(vec![DataType::Integer, DataType::Text]);
    /// assert!(source.is_compatible_with(&target)); // true
    /// ```
    ///
    /// #### Example 3: List Compatibility
    /// ```rust
    /// use baselard::component::DataType;
    /// let source = DataType::List(Box::new(DataType::Integer));
    /// let target = DataType::List(Box::new(DataType::Integer));
    /// assert!(source.is_compatible_with(&target)); // true
    /// ```
    ///
    /// #### Example 4: Nested List Compatibility
    /// ```rust
    /// use baselard::component::DataType;
    /// let source = DataType::List(Box::new(DataType::List(Box::new(DataType::Text))));
    /// let target = DataType::List(Box::new(DataType::List(Box::new(DataType::Text))));
    /// assert!(source.is_compatible_with(&target)); // true
    /// ```
    ///
    /// #### Example 5: Incompatible Types
    /// ```rust
    /// use baselard::component::DataType;
    /// let source = DataType::Integer;
    /// let target = DataType::Text;
    /// assert!(!source.is_compatible_with(&target)); // false
    /// ```
    pub fn is_compatible_with(&self, other: &DataType) -> bool {
        match (self, other) {
            (a, b) if a == b => true,

            (source_type, DataType::Union(target_types)) => target_types
                .iter()
                .any(|t| source_type.is_compatible_with(t)),

            (DataType::List(a), DataType::List(b)) => a.is_compatible_with(b),

            _ => false,
        }
    }
}

impl Data {
    pub fn as_integer(&self) -> Option<i32> {
        if let Data::Integer(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        if let Data::Text(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_list(&self) -> Option<&[Data]> {
        if let Data::List(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn get_type(&self) -> DataType {
        match self {
            Data::Null => DataType::Null,
            Data::Integer(_) => DataType::Integer,
            Data::Float(_) => DataType::Float,
            Data::Text(_) => DataType::Text,
            Data::List(items) => {
                if let Some(first) = items.first() {
                    DataType::List(Box::new(first.get_type()))
                } else {
                    DataType::List(Box::new(DataType::Integer))
                }
            }
            Data::Json(_) => DataType::Json,
            Data::OneConsumerChannel(_) => DataType::List(Box::new(DataType::Integer)),
        }
    }
}

pub type ComponentResult = Result<Data, DAGError>;

pub trait Component: Send + Sync + 'static {
    fn configure(config: Value) -> Self
    where
        Self: Sized;

    fn execute(&self, input: Data) -> ComponentResult;

    fn input_type(&self) -> DataType;

    fn output_type(&self) -> DataType;

    fn is_deferrable(&self) -> bool {
        false
    }
}

/// A configure on demand registry of components.
/// For performance might be better to cache configured components.
pub struct ComponentRegistry {
    components: HashMap<String, Arc<dyn Fn(Value) -> Box<dyn Component>>>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
        }
    }

    pub fn register<C: Component + 'static>(&mut self, name: &str) {
        self.components.insert(
            name.to_string(),
            Arc::new(|config| Box::new(C::configure(config)) as Box<dyn Component>),
        );
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Fn(Value) -> Box<dyn Component>>> {
        self.components.get(name)
    }
}

impl std::fmt::Debug for ComponentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentRegistry")
            .field("registered_components", &self.components.keys().collect::<Vec<_>>())
            .finish()
    }
}