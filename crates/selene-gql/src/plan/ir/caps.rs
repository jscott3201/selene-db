//! Planner implementation-defined limits.

use std::num::NonZeroUsize;

/// Planner implementation-defined limits.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ImplDefinedCaps {
    /// Maximum accepted variable-length quantifier upper bound.
    pub max_quantifier: u32,
    /// Fixed-point optimizer iteration cap.
    pub max_optimizer_iterations: u32,
    /// Maximum character length for character-string concatenation results.
    pub max_string_length: u32,
    /// Maximum byte length for byte-string concatenation results.
    pub max_byte_string_length: u32,
    /// Default maximum path length for future path execution.
    pub max_path_length: u32,
    /// Maximum list cardinality for constructed list values.
    pub max_list_length: u32,
    /// Maximum number of expand nodes WCO cycle detection will inspect.
    pub max_wco_traversal_nodes: u32,
    /// Maximum unique row keys a set operation may hold while counting rows.
    pub set_op_key_cap: NonZeroUsize,
    /// Maximum distinct groups a `GROUP BY` may materialize.
    pub group_by_key_cap: NonZeroUsize,
    // The four key-label-set cardinality bounds (IL003) are `pub(crate)`, NOT
    // `pub` like the other caps: the runtime can only represent a *singleton*
    // key label set, so these are a fixed internal invariant (min = max = 1),
    // not an embedder knob. Keeping them crate-private prevents an external
    // caller from mutating `ImplDefinedCaps::DEFAULT.node_key_label_set_max` past
    // 1 (which `#[non_exhaustive]` alone does NOT stop for public fields) and
    // committing an unrepresentable multi-label set. They become `pub` +
    // builder-configurable with multi-label runtime support (v1.3).
    /// Minimum cardinality of an explicit node-type key label set (ISO/IEC
    /// 39075:2024 IL003, §18.2 SR10). Fixed at 1.
    pub(crate) node_key_label_set_min: u32,
    /// Maximum cardinality of an explicit node-type key label set (ISO/IEC
    /// 39075:2024 IL003, §18.2 SR11). Fixed at 1 (singleton).
    pub(crate) node_key_label_set_max: u32,
    /// Minimum cardinality of an explicit edge-type key label set (ISO/IEC
    /// 39075:2024 IL003, §18.3 SR11). Fixed at 1.
    pub(crate) edge_key_label_set_min: u32,
    /// Maximum cardinality of an explicit edge-type key label set (ISO/IEC
    /// 39075:2024 IL003, §18.3 SR12). Fixed at 1 (singleton).
    pub(crate) edge_key_label_set_max: u32,
}

impl ImplDefinedCaps {
    /// Default maximum unique row keys a set operation may hold.
    pub const DEFAULT_SET_OP_KEY_CAP: usize = 1_000_000;

    /// Default maximum distinct groups a `GROUP BY` may materialize.
    pub const DEFAULT_GROUP_BY_KEY_CAP: usize = 1_000_000;

    /// Default maximum character length for character-string concatenation results.
    pub const DEFAULT_MAX_STRING_LENGTH: u32 = u32::MAX;

    /// Default maximum byte length for byte-string concatenation results.
    pub const DEFAULT_MAX_BYTE_STRING_LENGTH: u32 = u32::MAX;

    /// Default maximum cardinality for constructed list values.
    pub const DEFAULT_MAX_LIST_LENGTH: u32 = 1_000_000;

    /// The default implementation-defined caps as a `const`.
    ///
    /// Const-constructible so `const fn` callers (e.g. `Session::new`) can seed
    /// the embedder-facing default without invoking the non-`const`
    /// [`Default::default`]. [`Default`] delegates here, so the two never drift.
    pub const DEFAULT: Self = Self {
        max_quantifier: 100,
        max_optimizer_iterations: 8,
        max_string_length: Self::DEFAULT_MAX_STRING_LENGTH,
        max_byte_string_length: Self::DEFAULT_MAX_BYTE_STRING_LENGTH,
        max_path_length: 32,
        max_list_length: Self::DEFAULT_MAX_LIST_LENGTH,
        max_wco_traversal_nodes: 64,
        set_op_key_cap: NonZeroUsize::new(Self::DEFAULT_SET_OP_KEY_CAP)
            .expect("default set-op key cap is non-zero"),
        group_by_key_cap: NonZeroUsize::new(Self::DEFAULT_GROUP_BY_KEY_CAP)
            .expect("default group-by key cap is non-zero"),
        // IL003: singleton key label sets only. Multi-label is a deferred
        // feature (KeyLabelSetPolicy + containment identification), so the
        // default min == max == 1 rejects every non-singleton explicit key
        // label set with the spec-defined GQLSTATUS (§18.2 SR10/SR11, §18.3
        // SR11/SR12).
        node_key_label_set_min: 1,
        node_key_label_set_max: 1,
        edge_key_label_set_min: 1,
        edge_key_label_set_max: 1,
    };

    /// Return a copy with a different variable-length quantifier upper-bound
    /// cap (ISO IL018). Consulted by the plan-time quantifier gate.
    #[must_use]
    pub const fn with_max_quantifier(mut self, max_quantifier: u32) -> Self {
        self.max_quantifier = max_quantifier;
        self
    }

    /// Return a copy with a different maximum path length cap (ISO IL015).
    #[must_use]
    pub const fn with_max_path_length(mut self, max_path_length: u32) -> Self {
        self.max_path_length = max_path_length;
        self
    }

    /// Return a copy with a different character-string concat length cap (ISO IL013).
    #[must_use]
    pub const fn with_max_string_length(mut self, max_string_length: u32) -> Self {
        self.max_string_length = max_string_length;
        self
    }

    /// Return a copy with a different byte-string concat length cap (ISO IL013).
    #[must_use]
    pub const fn with_max_byte_string_length(mut self, max_byte_string_length: u32) -> Self {
        self.max_byte_string_length = max_byte_string_length;
        self
    }

    /// Return a copy with a different maximum list cardinality cap (ISO IL015).
    #[must_use]
    pub const fn with_max_list_length(mut self, max_list_length: u32) -> Self {
        self.max_list_length = max_list_length;
        self
    }

    /// Return the configured set-operation key cap.
    #[must_use]
    pub const fn set_op_key_cap(&self) -> usize {
        self.set_op_key_cap.get()
    }

    /// Return a copy with a different set-operation key cap.
    #[must_use]
    pub const fn with_set_op_key_cap(mut self, set_op_key_cap: NonZeroUsize) -> Self {
        self.set_op_key_cap = set_op_key_cap;
        self
    }

    /// Return the configured `GROUP BY` distinct-group cap.
    #[must_use]
    pub const fn group_by_key_cap(&self) -> usize {
        self.group_by_key_cap.get()
    }

    /// Return a copy with a different `GROUP BY` distinct-group cap.
    #[must_use]
    pub const fn with_group_by_key_cap(mut self, group_by_key_cap: NonZeroUsize) -> Self {
        self.group_by_key_cap = group_by_key_cap;
        self
    }

    // NOTE: the node/edge key-label-set cardinality bounds (IL003) are
    // deliberately NOT embedder-tunable in this release — unlike `max_quantifier`
    // / the key caps (which the runtime honors at any value), the runtime can
    // only represent a *singleton* key label set: `EdgeTypeDef.label` is a single
    // discriminator, and an empty explicit set has no element-type name. A
    // configurable bound would let `> 1` silently drop labels or `0` synthesize a
    // placeholder name. So the bounds are fixed at min = max = 1 via `DEFAULT` and
    // their fields are `pub(crate)` (not `pub`) — `#[non_exhaustive]` blocks only
    // struct-literal construction, not field mutation of a `DEFAULT`-derived
    // instance, so crate-private visibility is what actually keeps an embedder
    // from setting them. Configurable bounds land *with* multi-label runtime
    // support (v1.3); adding `with_{node,edge}_key_label_set_bounds` + making the
    // fields `pub` then is the atomic place to re-expose them.
}

impl Default for ImplDefinedCaps {
    fn default() -> Self {
        Self::DEFAULT
    }
}
