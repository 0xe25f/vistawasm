use vista_types::DebugView;

/// Return whether a debug view needs terrain metadata.
pub fn debug_view_needs_terrain(view: DebugView) -> bool {
  !matches!(view, DebugView::None)
}
