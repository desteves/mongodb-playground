<script lang="ts">
  import { onMount } from 'svelte';
  import { EditorView, basicSetup } from 'codemirror';
  import { Compartment, EditorState } from '@codemirror/state';
  import { keymap } from '@codemirror/view';
  import { redo as redoCommand, undo as undoCommand } from '@codemirror/commands';
  import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
  import { tags as t } from '@lezer/highlight';
  import { javascript } from '@codemirror/lang-javascript';

  let {
    value = $bindable(''),
    readonly = false,
    dark = false,
    onrun,
    onselection,
  }: {
    value?: string;
    readonly?: boolean;
    dark?: boolean;
    onrun?: () => void;
    /** Called with the highlighted text ('' when nothing is highlighted) whenever the selection changes. */
    onselection?: (text: string) => void;
  } = $props();

  // The theme can change while the editor is open (light / dark / auto toggle).
  const theme = new Compartment();

  // MongoDB palette: Evergreen surfaces, Spring Green strings, amber numbers.
  const darkColors = HighlightStyle.define([
    { tag: [t.keyword, t.operator, t.controlKeyword], color: '#c792ea' },
    { tag: [t.string, t.regexp], color: '#00ed64' },
    { tag: [t.number, t.bool, t.null, t.atom], color: '#ffc010' },
    { tag: [t.propertyName], color: '#6dc8ff' },
    { tag: [t.function(t.variableName), t.function(t.propertyName)], color: '#6dc8ff' },
    { tag: t.comment, color: '#889397', fontStyle: 'italic' },
  ]);
  const lightColors = HighlightStyle.define([
    { tag: [t.keyword, t.operator, t.controlKeyword], color: '#7b3fb5' },
    { tag: [t.string, t.regexp], color: '#00684a' },
    { tag: [t.number, t.bool, t.null, t.atom], color: '#944f01' },
    { tag: [t.propertyName], color: '#016bf8' },
    { tag: [t.function(t.variableName), t.function(t.propertyName)], color: '#016bf8' },
    { tag: t.comment, color: '#5c6c75', fontStyle: 'italic' },
  ]);
  const base = (isDark: boolean) =>
    EditorView.theme(
      {
        '&': { backgroundColor: 'var(--editor-bg)', color: 'var(--fg)' },
        '.cm-content': { caretColor: 'var(--accent)' },
        '.cm-gutters': { backgroundColor: 'var(--editor-gutter)', color: 'var(--muted)', border: 'none' },
        '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--editor-active)' },
        '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
          backgroundColor: isDark ? '#1f5d4a !important' : '#b6f0d6 !important',
        },
      },
      { dark: isDark },
    );
  const themeFor = (isDark: boolean) => [base(isDark), syntaxHighlighting(isDark ? darkColors : lightColors)];

  let host: HTMLDivElement;
  let view: EditorView | undefined;

  // Undo and redo go through the editor's own history, so clearing or loading a template can be undone.
  export function undo() {
    if (view) undoCommand(view);
  }
  export function redo() {
    if (view) redoCommand(view);
  }
  /** The highlighted text, or '' when nothing is highlighted. */
  export function getSelection(): string {
    if (!view) return '';
    const s = view.state.selection.main;
    return s.empty ? '' : view.state.sliceDoc(s.from, s.to);
  }

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: [
          basicSetup,
          // The shell look: JavaScript highlighting for db.coll.find({...}), unquoted keys, /regex/.
          // The server does the real parsing and reports errors with line and column.
          javascript(),
          // Cmd/Ctrl+Enter runs; Esc is left to the browser so keyboard users can tab out.
          keymap.of([{ key: 'Mod-Enter', run: () => (onrun?.(), true) }]),
          EditorState.readOnly.of(readonly),
          EditorView.editable.of(!readonly),
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) value = u.state.doc.toString();
            if (u.selectionSet || u.docChanged) {
              const s = u.state.selection.main;
              onselection?.(s.empty ? '' : u.state.sliceDoc(s.from, s.to));
            }
          }),
          theme.of(themeFor(dark)),
        ],
      }),
    });
    return () => view?.destroy();
  });

  $effect(() => {
    const d = dark;
    view?.dispatch({ effects: theme.reconfigure(themeFor(d)) });
  });

  // Push external changes (loading an example, new results) into the editor.
  $effect(() => {
    const next = value;
    if (view && next !== view.state.doc.toString()) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
    }
  });
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .editor :global(.cm-editor) {
    height: 100%;
    font-size: 14px;
    font-family: var(--font-mono);
  }
  .editor :global(.cm-scroller) {
    overflow: auto;
    font-family: var(--font-mono);
  }
  @media (max-width: 860px) {
    .editor :global(.cm-editor) {
      font-size: 16px;
    }
  }
</style>
