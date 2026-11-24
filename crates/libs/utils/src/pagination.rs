//! Cursor-based pagination primitives shared by repositories, use cases, and
//! HTTP handlers.
//!
//! A query orders rows by a [`Sort`] key and positions itself with a
//! [`Cursor`]; results come back as a [`Page`] whose [`Page::next`] cursor
//! points at the following page. None of these types perform I/O —
//! repositories translate them into their storage engine's constructs.

/// Sort direction for an ordered query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    /// Ascending order — the default.
    #[default]
    Asc,
    /// Descending order.
    Desc,
}

/// A sort key paired with its direction.
///
/// `K` is the sort key (for example, a domain-level sort-key enum);
/// repositories translate it into their storage engine's ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort<K> {
    /// Which column or field to sort by.
    pub key: K,
    /// Direction to sort in.
    pub order: SortOrder,
}

/// A pagination position and window size.
///
/// `C` is the caller's cursor type (typically decoded from an opaque token in
/// the HTTP layer). The default is a first page: `after: None`,
/// `limit: None`.
#[derive(Clone, Debug)]
pub struct Cursor<C> {
    /// Fetch rows strictly after this cursor, exclusive of it. `None` starts
    /// from the beginning.
    pub after: Option<C>,
    /// Maximum number of items to return. `None` lets the caller choose a
    /// default page size.
    pub limit: Option<u64>,
}

impl<C> Default for Cursor<C> {
    fn default() -> Self {
        Self {
            after: None,
            limit: None,
        }
    }
}

/// One page of results.
///
/// `T` is the item type and `C` the cursor type. A page with `next: None` is
/// the last page; the default is an empty page with no continuation.
#[derive(Clone, Debug)]
pub struct Page<T, C> {
    /// The items on this page.
    pub items: Vec<T>,
    /// Cursor for the following page, present only when more results may
    /// exist beyond this one.
    pub next: Option<C>,
}

impl<T, C> Default for Page<T, C> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next: None,
        }
    }
}
