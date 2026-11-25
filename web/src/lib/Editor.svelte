<script lang="ts">
  import { onMount } from 'svelte';
  import { EditorView, basicSetup } from 'codemirror';
  import { Compartment, EditorState } from '@codemirror/state';
  import { keymap } from '@codemirror/view';
  import { redo as redoCommand, undo as undoCommand } from '@codemirror/commands';
  import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
  import { tags as t } from '@lezer/highlight';
  import { javascript } from '@codemirror/lang-javascript';
  import { darkSyntax, lightSyntax } from './editorColors';

  let {
    value = $bindable(''),
    readonly = false,
    label = 'Editor',
    dark = false,
    onrun,
    onformat,
    onselection,
  }: {
    value?: string;
    readonly?: boolean;
    /** The accessible name of the text box (screen readers, and AI agents reading the accessibility tree). */
    label?: string;
    dark?: boolean;
    onrun?: () => void;
    /** Shift+Alt+F: tidy the text. */
    onformat?: () => void;
    /** Called with the highlighted text ('' when nothing is highlighted) and the line it starts on, whenever the selection changes. */
    onselection?: (text: string, startLine: number) => void;
  } = $props();

  // The theme can change while the editor is open (light / dark / auto toggle).
  const theme = new Compartment();

  // MongoDB palette: Evergreen surfaces, Spring Green strings, amber numbers (the colours live in editorColors.ts, where a
  // test keeps every one readable on every panel shade).
  const highlight = (c: typeof lightSyntax) =>
    HighlightStyle.define([
      { tag: [t.keyword, t.operator, t.controlKeyword], color: c.keyword },
      { tag: [t.string, t.regexp], color: c.string },
      { tag: [t.number, t.bool, t.null, t.atom], color: c.number },
      { tag: [t.propertyName], color: c.property },
      { tag: [t.function(t.variableName), t.function(t.propertyName)], color: c.property },
      { tag: t.comment, color: c.comment, fontStyle: 'italic' },
    ]);
  const darkColors = highlight(darkSyntax);
  const lightColors = highlight(lightSyntax);
  const base = (isDark: boolean) =>
    EditorView.theme(
      {
        '&': { backgroundColor: 'var(--editor-bg)', color: 'var(--fg)' },
        '.cm-content': { caretColor: 'var(--accent)' },
        '.cm-gutters': { backgroundColor: 'var(--editor-gutter)', color: 'var(--muted)', border: 'none' },
        // The selection is drawn BEHIND the text, so an opaque current-line background would hide a highlight on that
        // line: keep it translucent.
        '.cm-activeLine': { backgroundColor: isDark ? 'rgba(255, 255, 255, 0.05)' : 'rgba(0, 104, 74, 0.06)' },
        '.cm-activeLineGutter': { backgroundColor: 'var(--editor-active)' },
        // Highlighted text must stand out from every syntax colour (green strings, amber numbers) on every panel shade.
        // The blues are the editor-standard selection colours: clear on both themes, and a quieter shade when the editor
        // does not have focus so you can still see what is selected.
        '.cm-selectionBackground': { backgroundColor: isDark ? '#3a5068 !important' : '#cfe0f5 !important' },
        '&.cm-focused .cm-selectionBackground': { backgroundColor: isDark ? '#2b6cb0 !important' : '#8fbdf5 !important' },
        // Editors that cannot be edited (Results) use the browser's own selection.
        '.cm-content ::selection, .cm-line::selection': { backgroundColor: isDark ? '#2b6cb0' : '#8fbdf5' },
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
          keymap.of([
            { key: 'Mod-Enter', run: () => (onrun?.(), true) },
            { key: 'Shift-Alt-f', run: () => (onformat ? (onformat(), true) : false) },
          ]),
          EditorState.readOnly.of(readonly),
          EditorView.editable.of(!readonly),
          EditorView.lineWrapping,
          // Without a name the text box is announced as a bare "edit text", and an agent cannot tell the three apart.
          EditorView.contentAttributes.of({ 'aria-label': label, ...(readonly ? { 'aria-readonly': 'true' } : {}) }),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) value = u.state.doc.toString();
            if (u.selectionSet || u.docChanged) {
              const s = u.state.selection.main;
              onselection?.(s.empty ? '' : u.state.sliceDoc(s.from, s.to), u.state.doc.lineAt(s.from).number);
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
