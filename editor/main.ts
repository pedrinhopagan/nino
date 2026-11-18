import { Editor, Extension, InputRule, wrappingInputRule, type JSONContent } from "@tiptap/core";
import { newlineInCode, splitBlockKeepMarks } from "@tiptap/pm/commands";
import type { NodeType, ResolvedPos } from "@tiptap/pm/model";
import { NodeSelection, TextSelection } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";
import StarterKit from "@tiptap/starter-kit";

import {
  convertCodeFenceOnEnter,
  exitCodeBlockOnClosingFence,
  exitCodeBlockOnTrailingBlankLines,
  indentCodeBlock,
  indentedNewlineInCodeBlock,
  selectionInOneCodeBlock,
} from "./t3/composer-code-block";
import {
  collapseShownMarkdownSource,
  ComposerMarkdownSourceExtension,
} from "./t3/composer-markdown-source";
import { MARKDOWN_SOURCE_MARK } from "./t3/composer-rich-text";
import {
  buildTiptapContent,
  ComposerBlockExtensions,
  ComposerBoldExtension,
  ComposerCodeBlockExtension,
  ComposerCodeExtension,
  ComposerItalicExtension,
  ComposerListExtensions,
  ComposerStrikeExtension,
  ComposerTaskItemExtension,
  ComposerTaskListExtension,
  convertBulletItemToTask,
  serializeEditorDoc,
  splitOrLiftListItem,
  stepCaretAcrossStyledEdge,
  surroundSelectionWithMark,
} from "./t3/composer-rich-text-doc";
import {
  COMPOSER_UNDO_GROUP_DELAY,
  type ComposerChangeKind,
  groupUndoByChangeKind,
  markAsClipboardEdit,
} from "./t3/composer-undo-grouping";

const SURROUND_CLOSE: Record<string, string> = {
  "(": ")",
  "[": "]",
  "{": "}",
  "'": "'",
  '"': '"',
  "“": "”",
  "`": "`",
  "<": ">",
  "«": "»",
  "*": "*",
  _: "_",
};

function hasAncestor($pos: ResolvedPos, name: string): boolean {
  for (let depth = $pos.depth; depth > 0; depth -= 1) {
    if ($pos.node(depth).type.name === name) return true;
  }
  return false;
}

function isInCodeBlock(view: EditorView): boolean {
  return view.state.selection.$from.parent.type.spec.code === true;
}

const blockquoteInputRule = new InputRule({
  find: /^>(\s)$/,
  handler: ({ state, range, match, chain }) => {
    const $from = state.doc.resolve(range.from);
    if ($from.parent.type.name !== "paragraph" || $from.depth !== 1) return null;
    chain()
      .deleteRange(range)
      .wrapIn("blockquote", { prefix: `>${match[1] ?? " "}` })
      .run();
    return undefined;
  },
});

const horizontalRuleInputRule = new InputRule({
  find: /^(---|\*\*\*|___)\s?$/,
  handler: ({ state, range, match, chain }) => {
    const source = match[0] ?? "---";
    if (!source.startsWith("---") && !/\s$/.test(source)) return null;
    const $from = state.doc.resolve(range.from);
    if ($from.parent.type.name !== "paragraph" || $from.depth !== 1) return null;
    chain()
      .deleteRange(range)
      .setHorizontalRule()
      .command(({ tr }) => {
        const $pos = tr.selection.$from;
        const index = $pos.index(0) - 1;
        if (index < 0) return true;
        const rulePos = $pos.posAtIndex(index, 0);
        const rule = tr.doc.nodeAt(rulePos);
        if (rule?.type.name === "horizontalRule") {
          tr.setNodeMarkup(rulePos, undefined, { ...rule.attrs, source });
        }
        return true;
      })
      .run();
    return undefined;
  },
});

const headingInputRule = new InputRule({
  find: /^(#{1,6})(\s)$/,
  handler: ({ state, range, match, chain }) => {
    const $from = state.doc.resolve(range.from);
    if ($from.parent.type.name !== "paragraph" || $from.depth !== 1) return null;
    chain()
      .deleteRange(range)
      .setNode("heading", { level: match[1]?.length ?? 1, space: match[2] ?? " " })
      .run();
    return undefined;
  },
});

function listMarkerInputRule(find: RegExp, listType: "bulletList" | "orderedList"): InputRule {
  return new InputRule({
    find,
    handler: ({ state, range, match, chain }) => {
      const marker = match.groups?.marker ?? "-";
      const space = match.groups?.space ?? " ";
      const carried = match.groups?.carried ?? "";
      const $from = state.doc.resolve(range.from);
      if ($from.parent.type.name !== "paragraph" || $from.depth !== 1) return null;
      const command = chain()
        .deleteRange(range)
        .wrapInList(
          listType,
          listType === "orderedList" ? { start: Number.parseInt(marker, 10) || 1 } : {},
        )
        .updateAttributes("listItem", { marker, space });
      (carried ? command.insertContent(carried) : command).run();
      return undefined;
    },
  });
}

const bulletToTaskInputRule = new InputRule({
  find: /^\[([ xX])\] $/,
  handler: ({ state, range, match, chain }) => {
    const $from = state.doc.resolve(range.from);
    const item = $from.node(-1);
    if ($from.parent.type.name !== "paragraph" || item?.type.name !== "listItem") return null;
    if (!["-", "*", "+"].includes((item.attrs as { marker?: string }).marker ?? "")) return null;
    const checked = (match[1] ?? " ").toLowerCase() === "x";
    chain()
      .command(({ tr }) => {
        convertBulletItemToTask(tr, range.from, range.to, checked);
        return true;
      })
      .run();
    return undefined;
  },
});

function taskInputRule(type: NodeType): InputRule {
  const rule = wrappingInputRule({
    find: /^- \[([ xX])\] $/,
    type,
    getAttributes: (match) => ({ checked: match[1]?.toLowerCase() === "x" }),
  });
  return new InputRule({
    find: rule.find,
    handler: (props) =>
      props.state.doc.resolve(props.range.from).depth === 1 ? rule.handler(props) : null,
  });
}

const UndoGroupingExtension = Extension.create<
  Record<string, never>,
  { previous: ComposerChangeKind | null }
>({
  name: "composer-undo-grouping",
  addStorage() {
    return { previous: null };
  },
  dispatchTransaction({ transaction, next }) {
    this.storage.previous = groupUndoByChangeKind(transaction, this.storage.previous);
    next(transaction);
  },
});

const noSkills = () => null;

function insertMarkdownParagraphs(
  value: string,
  blocks: boolean,
  insertContent: (content: JSONContent[] | JSONContent) => void,
): void {
  const content = buildTiptapContent(value, noSkills, { styling: true, blocks }) as JSONContent[];
  if (content.length === 1 && content[0]?.type === "paragraph") {
    const inline = content[0].content ?? [];
    if (inline.length > 0) insertContent(inline);
    return;
  }
  insertContent(content);
}

type Entry = { clock: string; text: string; image: boolean };

declare global {
  interface Window {
    webkit?: { messageHandlers: { nino: { postMessage: (message: string) => void } } };
    NINO: { clock: string; entries: Entry[] };
    ninoSubmit: () => void;
    ninoArm: () => void;
  }
}

const root = document.getElementById("editor")!;
const title = document.getElementById("title")!;
const status = document.getElementById("state")!;
const list = document.getElementById("entries")!;
const { clock, entries } = window.NINO;
let editing: number | null = null;
let submitted = false;
let disarm = 0;

function post(message: string) {
  if (window.webkit) window.webkit.messageHandlers.nino.postMessage(message);
  else console.log(message);
}

function el(tag: string, className: string, text = ""): HTMLElement {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = text;
  return node;
}

const editor: Editor = new Editor({
  element: root,
  autofocus: true,
  extensions: [
    StarterKit.configure({
      blockquote: false,
      bulletList: false,
      codeBlock: false,
      heading: false,
      horizontalRule: false,
      listItem: false,
      link: false,
      orderedList: false,
      underline: false,
      dropcursor: false,
      gapcursor: false,
      trailingNode: false,
      code: false,
      listKeymap: false,
      undoRedo: { newGroupDelay: COMPOSER_UNDO_GROUP_DELAY },
      italic: false,
      bold: false,
      strike: false,
    }),
    UndoGroupingExtension,
    ComposerMarkdownSourceExtension,
    ComposerItalicExtension,
    ComposerBoldExtension,
    ComposerStrikeExtension,
    ComposerCodeExtension,
    ComposerCodeBlockExtension.extend({
      addInputRules() {
        return [];
      },
    }),
    ...ComposerBlockExtensions.map((extension) =>
      extension.name === "blockquote"
        ? extension.extend({ addInputRules: () => [blockquoteInputRule] })
        : extension.name === "horizontalRule"
          ? extension.extend({ addInputRules: () => [horizontalRuleInputRule] })
          : extension.extend({ addInputRules: () => [headingInputRule] }),
    ),
    ...ComposerListExtensions.map((extension) =>
      extension.name === "listItem"
        ? extension
        : extension.extend({
            addInputRules() {
              return this.name === "bulletList"
                ? [
                    listMarkerInputRule(/^(?<marker>[*+])(?<space>\s)$/, "bulletList"),
                    listMarkerInputRule(
                      /^(?<marker>-)(?<space>[ \t]+)(?<carried>[^\s[])$/,
                      "bulletList",
                    ),
                  ]
                : [listMarkerInputRule(/^(?<marker>\d+[.)])(?<space>\s)$/, "orderedList")];
            },
          }),
    ),
    ComposerTaskListExtension,
    ComposerTaskItemExtension.extend({
      addInputRules() {
        return [taskInputRule(this.type), bulletToTaskInputRule];
      },
    }),
  ],
  editorProps: {
    attributes: { class: "composer-tiptap", "aria-label": "Comentário", spellcheck: "false" },
    handleKeyDown: (view, event) => {
      if (
        ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key) &&
        !event.shiftKey &&
        !event.altKey &&
        !event.metaKey &&
        !event.ctrlKey &&
        !event.isComposing &&
        view.state.selection.empty
      ) {
        const { $from } = view.state.selection;
        const direction = event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1 : 1;
        const horizontal = event.key === "ArrowLeft" || event.key === "ArrowRight";
        const atDocumentEdge =
          $from.pos ===
          (direction === -1
            ? TextSelection.atStart(view.state.doc).from
            : TextSelection.atEnd(view.state.doc).to);
        const step =
          horizontal || atDocumentEdge ? stepCaretAcrossStyledEdge(view.state, direction) : null;
        if (step) {
          event.preventDefault();
          view.dispatch(step);
          return true;
        }
      }
      if (event.key === "Enter" && (event.isComposing || event.keyCode === 229)) {
        event.stopPropagation();
        return true;
      }
      if (event.key === "Enter" && event.target instanceof HTMLInputElement) {
        event.preventDefault();
        return true;
      }
      if (
        event.key === "Tab" &&
        !event.metaKey &&
        !event.ctrlKey &&
        selectionInOneCodeBlock(view.state)
      ) {
        event.preventDefault();
        return indentCodeBlock(view.state, event.shiftKey ? "out" : "in", (tr) =>
          view.dispatch(tr),
        );
      }
      if (event.key === "Enter" && !event.metaKey && !event.ctrlKey && !event.isComposing) {
        if (selectionInOneCodeBlock(view.state)) {
          event.preventDefault();
          const dispatch = (tr: typeof view.state.tr) => view.dispatch(tr.scrollIntoView());
          return (
            exitCodeBlockOnClosingFence(view.state, dispatch) ||
            exitCodeBlockOnTrailingBlankLines(view) ||
            indentedNewlineInCodeBlock(view.state, dispatch) ||
            newlineInCode(view.state, dispatch)
          );
        }
        if (convertCodeFenceOnEnter(view.state, (tr) => view.dispatch(tr))) {
          event.preventDefault();
          return true;
        }
      }
      if (event.key !== "Enter") return false;
      const inList = editor.isActive("taskItem") || editor.isActive("listItem");
      event.preventDefault();
      if (!event.shiftKey && (!inList || event.ctrlKey)) {
        window.ninoSubmit();
        return true;
      }
      if (inList && splitOrLiftListItem(editor, collapseShownMarkdownSource)) return true;
      if (
        hasAncestor(view.state.selection.$from, "blockquote") &&
        view.state.selection.$from.parent.content.size === 0 &&
        editor.commands.lift("blockquote")
      ) {
        return true;
      }
      return editor
        .chain()
        .command(collapseShownMarkdownSource)
        .command(({ state, dispatch }) => splitBlockKeepMarks(state, dispatch))
        .scrollIntoView()
        .run();
    },
    handleTextInput: (view, from, to, text) => {
      if (view.composing || text.length !== 1) return false;
      const closer = SURROUND_CLOSE[text];
      const { selection } = view.state;
      if (
        !closer ||
        !(selection instanceof TextSelection) ||
        selection.empty ||
        selection.from !== from ||
        selection.to !== to
      ) {
        return false;
      }
      let touchesSpecial = false;
      view.state.doc.nodesBetween(from, to, (node) => {
        if (
          (node.isAtom && node.isInline && !node.isText) ||
          (node.isText && node.marks.some((mark) => mark.type.name !== MARKDOWN_SOURCE_MARK))
        ) {
          touchesSpecial = true;
          return false;
        }
        return true;
      });
      if (touchesSpecial) return false;
      const markTr = surroundSelectionWithMark(view.state, from, to, text);
      if (markTr) {
        view.dispatch(markTr);
        return true;
      }
      const tr = view.state.tr.insertText(closer, to).insertText(text, from);
      tr.setSelection(TextSelection.create(tr.doc, from + text.length, to + text.length));
      view.dispatch(tr);
      return true;
    },
    handlePaste: (view, event) => {
      const pastedText = event.clipboardData?.getData("text/plain").replace(/\r\n?/g, "\n");
      if (!pastedText) return false;
      event.preventDefault();
      const { from, to, $from } = view.state.selection;
      const sourceType = view.state.schema.marks[MARKDOWN_SOURCE_MARK];
      if (
        isInCodeBlock(view) ||
        (sourceType?.isInSet($from.marks()) && !pastedText.includes("\n"))
      ) {
        view.dispatch(
          markAsClipboardEdit(
            view.state.tr.insertText(pastedText, from, to),
            "paste",
          ).scrollIntoView(),
        );
        return true;
      }
      const nested = ["listItem", "taskItem", "blockquote"].some((name) =>
        hasAncestor($from, name),
      );
      insertMarkdownParagraphs(pastedText, !nested, (content) => {
        editor
          .chain()
          .command(({ tr }) => {
            markAsClipboardEdit(tr, "paste");
            return true;
          })
          .command(collapseShownMarkdownSource)
          .insertContent(content)
          .command(({ tr }) => {
            const { selection } = tr;
            if (!(selection instanceof NodeSelection) || !selection.node.isBlock) return true;
            if (!tr.doc.resolve(selection.to).nodeAfter?.isTextblock) {
              tr.insert(selection.to, tr.doc.type.schema.nodes.paragraph!.create());
            }
            tr.setSelection(TextSelection.create(tr.doc, selection.to + 1));
            return true;
          })
          .scrollIntoView()
          .run();
      });
      return true;
    },
  },
  onUpdate: syncEmpty,
});

function syncEmpty() {
  root.dataset.empty = String(editor.isEmpty);
}

function idle() {
  status.classList.remove("warn");
  status.textContent = editing === null ? "gravação pausada" : "editando";
}

function render() {
  title.textContent =
    editing === null
      ? `Anotação ${entries.length + 1} · ${clock}`
      : `Anotação ${editing + 1} · ${entries[editing]!.clock}`;
  if (!status.classList.contains("warn")) idle();
  list.replaceChildren(
    ...entries.map((entry, i) => {
      const item = el("li", i === editing ? "entry active" : "entry");
      const open = el("button", "open");
      const head = el("span", "head");
      head.append(el("b", "", `Anotação ${i + 1}`), entry.clock);
      if (entry.image) head.append(el("span", "tag", "imagem"));
      open.append(head, el("span", "excerpt", entry.text));
      open.addEventListener("click", () => edit(i));
      item.append(open);
      if (i !== editing) {
        const remove = el("button", "delete", "×");
        remove.title = "Apagar";
        remove.addEventListener("click", () => drop(i));
        item.append(remove);
      }
      return item;
    }),
  );
}

function edit(i: number) {
  if (!submitted && i !== editing) {
    editing = i;
    const content = buildTiptapContent(entries[i]!.text, noSkills, {
      styling: true,
      blocks: true,
    }) as JSONContent[];
    editor.commands.setContent({
      type: "doc",
      content: content.length > 0 ? content : [{ type: "paragraph" }],
    });
    syncEmpty();
    post(`edit ${i + 1}`);
    render();
  }
  editor.commands.focus("end");
}

function drop(i: number) {
  if (!submitted && i !== editing) {
    entries.splice(i, 1);
    if (editing !== null && editing > i) editing -= 1;
    post(`delete ${i + 1}`);
    render();
  }
  editor.commands.focus();
}

window.ninoSubmit = () => {
  if (submitted) return;
  submitted = true;
  post(`submit\n${serializeEditorDoc(editor.state.doc).value.trim()}`);
};

window.ninoArm = () => {
  status.textContent = "Esc de novo cancela a gravação";
  status.classList.add("warn");
  clearTimeout(disarm);
  disarm = window.setTimeout(idle, 1500);
};

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  event.preventDefault();
  post("esc");
});

if (entries.length === 0) document.body.classList.add("solo");
syncEmpty();
render();
