<script lang="ts">
  // Small line icons (24x24 grid, drawn with the current text colour) so buttons stay readable in both themes.
  type Name =
    | 'undo'
    | 'redo'
    | 'chevron-up'
    | 'chevron-down'
    | 'chevrons-left'
    | 'chevrons-right'
    | 'x'
    | 'copy'
    | 'download'
    | 'play'
    | 'clock'
    | 'columns'
    | 'rows'
    | 'square';

  let { name, size = 18 }: { name: Name; size?: number } = $props();

  // Constant markup, never user input.
  const PATHS: Record<Name, string> = {
    undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
    redo: '<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"/>',
    'chevron-up': '<path d="m18 15-6-6-6 6"/>',
    'chevron-down': '<path d="m6 9 6 6 6-6"/>',
    'chevrons-left': '<path d="m11 17-5-5 5-5"/><path d="m18 17-5-5 5-5"/>',
    'chevrons-right': '<path d="m6 17 5-5-5-5"/><path d="m13 17 5-5-5-5"/>',
    x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    copy: '<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2"/>',
    download: '<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>',
    play: '<path d="M6 4v16l14-8z" fill="currentColor"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    columns: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M12 3v18"/>',
    rows: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 12h18"/>',
    square: '<rect width="18" height="18" x="3" y="3" rx="2"/>',
  };
</script>

<svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  {@html PATHS[name]}
</svg>
