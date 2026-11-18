import { Editor, Extension, wrappingInputRule, type JSONContent } from "@tiptap/core";
import { Code } from "@tiptap/extension-code";
import { TaskItem } from "@tiptap/extension-task-item";
import { TaskList } from "@tiptap/extension-task-list";
import { splitBlockKeepMarks } from "@tiptap/pm/commands";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { Plugin, PluginKey, TextSelection, type Selection } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import StarterKit from "@tiptap/starter-kit";

type RichTextMark = "bold" | "italic" | "strike" | "code";

interface RichTextSpan {
  text: string;
  marks: RichTextMark[];
}

const RICH_TEXT_DELIMITERS: Record<RichTextMark, string> = {
  bold: "**",
  italic: "*",
  strike: "~~",
  code: "`",
};

const MARK_NESTING_ORDER: RichTextMark[] = ["strike", "bold", "italic", "code"];

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

function pushSpan(spans: RichTextSpan[], text: string, marks: RichTextMark[]): void {
  if (!text) return;
  const last = spans[spans.length - 1];
  if (
    last &&
    last.marks.length === marks.length &&
    last.marks.every((mark, index) => mark === marks[index])
  ) {
    last.text += text;
  } else {
    spans.push({ text, marks });
  }
}

function parseInlineMarkdown(text: string): RichTextSpan[] {
  const root: RichTextSpan[] = [];
  const stack: { delimiter: string; mark: RichTextMark; spans: RichTextSpan[] }[] = [];
  const current = () => stack.at(-1)?.spans ?? root;
  let index = 0;
  while (index < text.length) {
    const char = text[index]!;
    if (char === "\\") {
      pushSpan(current(), text.slice(index, index + 2), []);
      index += 2;
      continue;
    }
    if (char === "`") {
      const run = text.slice(index).match(/^`+/)![0];
      const close = text.indexOf(run, index + run.length);
      if (
        run.length === 1 &&
        close > index + 1 &&
        !text.slice(index, close).includes("\n") &&
        text[close + 1] !== "`"
      ) {
        pushSpan(current(), text.slice(index + 1, close), ["code"]);
        index = close + 1;
      } else {
        pushSpan(current(), run, []);
        index += run.length;
      }
      continue;
    }
    if (char !== "*" && char !== "_" && char !== "~") {
      pushSpan(current(), char, []);
      index += 1;
      continue;
    }
    let end = index;
    while (text[end] === char) end += 1;
    const before = text[index - 1] ?? "";
    const after = text[end] ?? "";
    const canClose = before !== "" && !/\s/.test(before) && (char !== "_" || !/\w/.test(after));
    const canOpen = after !== "" && !/\s/.test(after) && (char !== "_" || !/\w/.test(before));
    while (index < end) {
      const top = stack.at(-1);
      const opensNestedBold =
        top?.mark === "italic" &&
        end - index === 2 &&
        canOpen &&
        (char === "*" || char === "_") &&
        !stack.some((frame) => frame.mark === "bold");
      if (
        !opensNestedBold &&
        canClose &&
        top &&
        text.startsWith(top.delimiter, index) &&
        index + top.delimiter.length <= end
      ) {
        stack.pop();
        for (const span of top.spans) pushSpan(current(), span.text, [top.mark, ...span.marks]);
        index += top.delimiter.length;
      } else if (canOpen && (char !== "~" || end - index >= 2)) {
        const length = char === "~" || end - index >= 2 ? 2 : 1;
        const mark = char === "~" ? "strike" : length === 2 ? "bold" : "italic";
        if (stack.some((frame) => frame.mark === mark)) {
          pushSpan(current(), text.slice(index, end), []);
          index = end;
          continue;
        }
        stack.push({ delimiter: char.repeat(length), mark, spans: [] });
        index += length;
      } else {
        pushSpan(current(), text.slice(index, end), []);
        index = end;
      }
    }
  }
  while (stack.length > 0) {
    const frame = stack.pop()!;
    pushSpan(current(), frame.delimiter, []);
    for (const span of frame.spans) pushSpan(current(), span.text, span.marks);
  }
  return root;
}

const NinoCode = Code.extend({ excludes: "code" });

const NinoTaskItem = TaskItem.extend({
  addAttributes() {
    return {
      ...this.parent?.(),
      indent: { default: "" },
      markerSpace: { default: " " },
      contentSpace: { default: null },
    };
  },
  addInputRules() {
    return [
      wrappingInputRule({
        find: /^- \[([ xX])\] $/,
        type: this.type,
        getAttributes: (match) => ({ checked: match[1]?.toLowerCase() === "x" }),
      }),
    ];
  },
}).configure({ nested: true });

interface TaskLinePrefix {
  indent: string;
  checked: boolean;
  markerSpace: string;
  contentSpace: string;
}

interface PendingTaskItem extends TaskLinePrefix {
  content: JSONContent[];
  children: PendingTaskItem[];
}

function parseTaskPrefix(head: string): { prefix: TaskLinePrefix; markerLength: number } | null {
  const match = head.match(/^([ \t]*)-([ \t]+)\[([ xX])\]([ \t]*)/);
  if (!match) return null;
  const after = head.slice(match[0].length);
  if (after.length > 0 && !match[4]) return null;
  return {
    prefix: {
      indent: match[1] ?? "",
      checked: (match[3] ?? " ").toLowerCase() === "x",
      markerSpace: match[2]!,
      contentSpace: match[4]!,
    },
    markerLength: match[0].length,
  };
}

function taskListJson(items: PendingTaskItem[]): JSONContent {
  return {
    type: "taskList",
    content: items.map((item) => ({
      type: "taskItem",
      attrs: {
        checked: item.checked,
        indent: item.indent,
        markerSpace: item.markerSpace,
        contentSpace: item.contentSpace,
      },
      content: [
        { type: "paragraph", content: item.content },
        ...(item.children.length > 0 ? [taskListJson(item.children)] : []),
      ],
    })),
  };
}

function textJsonForSpan(text: string, marks: RichTextMark[]): JSONContent {
  const json: JSONContent = { type: "text", text };
  if (marks.length > 0) {
    json.marks = [...marks]
      .sort((a, b) => MARK_NESTING_ORDER.indexOf(a) - MARK_NESTING_ORDER.indexOf(b))
      .map((mark) => ({ type: mark }));
  }
  return json;
}

function buildTiptapContent(value: string): JSONContent[] {
  const lines = value.split("\n").map((line) => {
    const parsed = parseTaskPrefix(line);
    const content = parsed ? line.slice(parsed.markerLength) : line;
    const inline = parseInlineMarkdown(content).map((span) => textJsonForSpan(span.text, span.marks));
    return { task: parsed?.prefix ?? null, inline };
  });
  const blocks: JSONContent[] = [];
  let stack: { indent: string; items: PendingTaskItem[] }[] = [];
  const flushTasks = () => {
    if (stack.length > 0) {
      blocks.push(taskListJson(stack[0]!.items));
      stack = [];
    }
  };
  for (const line of lines) {
    if (!line.task) {
      flushTasks();
      blocks.push({ type: "paragraph", content: line.inline });
      continue;
    }
    const item: PendingTaskItem = { ...line.task, content: line.inline, children: [] };
    for (;;) {
      const top = stack[stack.length - 1];
      if (!top) {
        stack.push({ indent: item.indent, items: [] });
        continue;
      }
      if (top.indent === item.indent) {
        top.items.push(item);
        break;
      }
      if (top.indent !== "" && !item.indent.startsWith(top.indent)) {
        if (stack.length > 1) {
          stack.pop();
          continue;
        }
        top.indent = item.indent;
        top.items.push(item);
        break;
      }
      const parent = top.items[top.items.length - 1];
      if (!parent) {
        top.items.push(item);
        break;
      }
      parent.children.push(item);
      stack.push({ indent: item.indent, items: parent.children });
      break;
    }
  }
  flushTasks();
  return blocks;
}

interface RichRun {
  pmPos: number;
  docLen: number;
  mdStart: number;
  mdLen: number;
  openLen: number;
  closeLen: number;
}

interface RichAccumulator {
  runs: RichRun[];
  value: string;
}

function pushBreakRun(acc: RichAccumulator, position?: number): void {
  const previous = acc.runs[acc.runs.length - 1];
  const pmPos = position ?? (previous ? previous.pmPos + previous.docLen : 1);
  acc.runs.push({ pmPos, docLen: 1, mdStart: acc.value.length, mdLen: 1, openLen: 0, closeLen: 0 });
  acc.value += "\n";
}

function markNames(node: ProseMirrorNode): RichTextMark[] {
  return node.marks
    .map((mark) => mark.type.name)
    .filter((name): name is RichTextMark => MARK_NESTING_ORDER.includes(name as RichTextMark));
}

function appendInlineRuns(container: ProseMirrorNode, contentStart: number, acc: RichAccumulator): void {
  const children: ProseMirrorNode[] = [];
  container.forEach((child) => {
    if (!child.isText || child.marks.some((mark) => mark.type.name === "code")) {
      children.push(child);
      return;
    }
    const text = child.text!;
    const start = text.length - text.trimStart().length;
    const end = Math.max(start, text.trimEnd().length);
    let offset = 0;
    for (const boundary of [start, end, text.length]) {
      if (boundary > offset) children.push(child.cut(offset, boundary));
      offset = boundary;
    }
  });
  for (const mark of MARK_NESTING_ORDER) {
    if (mark === "code") continue;
    for (const direction of [1, -1]) {
      let hasContent = false;
      for (
        let index = direction === 1 ? 0 : children.length - 1;
        index >= 0 && index < children.length;
        index += direction
      ) {
        const child = children[index]!;
        if (child.type.name === "hardBreak" || !child.marks.some((item) => item.type.name === mark)) {
          hasContent = false;
        } else if (
          child.isText &&
          !child.marks.some((item) => item.type.name === "code") &&
          /^\s+$/.test(child.text!)
        ) {
          if (!hasContent) children[index] = child.mark(child.marks.filter((item) => item.type.name !== mark));
        } else {
          hasContent = true;
        }
      }
    }
  }
  const markEnds = new Map<RichTextMark, number>();
  const orderedMarks: RichTextMark[][] = [];
  for (let index = children.length - 1; index >= 0; index -= 1) {
    const child = children[index]!;
    const marks = child.type.name === "hardBreak" ? [] : markNames(child);
    for (const mark of MARK_NESTING_ORDER) {
      if (!marks.includes(mark)) markEnds.delete(mark);
      else if (!markEnds.has(mark)) markEnds.set(mark, index);
    }
    orderedMarks[index] = marks.sort(
      (a, b) =>
        markEnds.get(b)! - markEnds.get(a)! || MARK_NESTING_ORDER.indexOf(a) - MARK_NESTING_ORDER.indexOf(b),
    );
  }
  for (let index = 1; index < orderedMarks.length; index += 1) {
    const marks = orderedMarks[index]!;
    const retained: RichTextMark[] = [];
    for (const mark of orderedMarks[index - 1]!) {
      if (!marks.includes(mark)) break;
      retained.push(mark);
    }
    orderedMarks[index] = [...retained, ...marks.filter((mark) => !retained.includes(mark))];
  }
  const commonLength = (left: RichTextMark[], right: RichTextMark[]) => {
    let index = 0;
    while (index < left.length && left[index] === right[index]) index += 1;
    return index;
  };
  let inlineOffset = 0;
  children.forEach((child, index) => {
    const pmPos = contentStart + inlineOffset;
    inlineOffset += child.nodeSize;
    if (child.type.name === "hardBreak") {
      pushBreakRun(acc, pmPos);
      return;
    }
    const marks = orderedMarks[index]!;
    const open = marks
      .slice(commonLength(marks, orderedMarks[index - 1] ?? []))
      .map((mark) => RICH_TEXT_DELIMITERS[mark])
      .join("");
    const close = marks
      .slice(commonLength(marks, orderedMarks[index + 1] ?? []))
      .toReversed()
      .map((mark) => RICH_TEXT_DELIMITERS[mark])
      .join("");
    const source = child.text ?? "";
    const mdText = open + source + close;
    acc.runs.push({
      pmPos,
      docLen: source.length,
      mdStart: acc.value.length,
      mdLen: mdText.length,
      openLen: open.length,
      closeLen: close.length,
    });
    acc.value += mdText;
  });
  if (children.length === 0) {
    acc.runs.push({ pmPos: contentStart, docLen: 0, mdStart: acc.value.length, mdLen: 0, openLen: 0, closeLen: 0 });
  }
}

function walkTaskList(list: ProseMirrorNode, listStart: number, acc: RichAccumulator): void {
  let itemPos = listStart + 1;
  let firstItem = true;
  list.content.forEach((item) => {
    if (!firstItem) pushBreakRun(acc);
    firstItem = false;
    const itemContentStart = itemPos + 1;
    const first = item.firstChild;
    const empty = first?.type.name === "paragraph" && first.content.childCount === 0;
    const attrs = item.attrs as Record<string, unknown>;
    const indent = typeof attrs.indent === "string" ? attrs.indent : "";
    const markerSpace = typeof attrs.markerSpace === "string" ? attrs.markerSpace : " ";
    const contentSpace =
      typeof attrs.contentSpace === "string" ? attrs.contentSpace || (empty ? "" : " ") : empty ? "" : " ";
    const prefix = `${indent}-${markerSpace}[${attrs.checked === true ? "x" : " "}]${contentSpace}`;
    acc.runs.push({
      pmPos: itemContentStart + 1,
      docLen: 0,
      mdStart: acc.value.length,
      mdLen: prefix.length,
      openLen: 0,
      closeLen: 0,
    });
    acc.value += prefix;
    let childPos = itemContentStart;
    let firstBlock = true;
    item.content.forEach((child) => {
      if (!firstBlock) pushBreakRun(acc);
      firstBlock = false;
      if (child.type.name === "taskList") walkTaskList(child, childPos, acc);
      else if (child.type.name === "paragraph") appendInlineRuns(child, childPos + 1, acc);
      childPos += child.nodeSize;
    });
    itemPos += item.nodeSize;
  });
}

function serializeEditorDoc(doc: ProseMirrorNode): RichAccumulator {
  const acc: RichAccumulator = { runs: [], value: "" };
  let pmBlockStart = 0;
  doc.content.forEach((block, _, blockIndex) => {
    if (blockIndex > 0) pushBreakRun(acc);
    if (block.type.name === "taskList") walkTaskList(block, pmBlockStart, acc);
    else if (block.type.name === "paragraph") appendInlineRuns(block, pmBlockStart + 1, acc);
    pmBlockStart += block.nodeSize;
  });
  return acc;
}

interface StyledRange {
  from: number;
  to: number;
  markers: { at: number; side: number; text: string }[];
}

function collectStyledRanges(doc: ProseMirrorNode): StyledRange[] {
  const ranges: StyledRange[] = [];
  const map = serializeEditorDoc(doc);
  let range: StyledRange | null = null;
  let openLength = 0;
  for (const run of map.runs) {
    if (run.openLen > 0) {
      range ??= { from: run.pmPos, to: run.pmPos, markers: [] };
      range.markers.push({ at: run.pmPos, side: -1, text: map.value.slice(run.mdStart, run.mdStart + run.openLen) });
    }
    if (range === null) continue;
    range.to = run.pmPos + run.docLen;
    if (run.closeLen > 0) {
      const end = run.mdStart + run.mdLen;
      range.markers.push({ at: range.to, side: -2, text: map.value.slice(end - run.closeLen, end) });
    }
    openLength += run.openLen - run.closeLen;
    if (openLength === 0) {
      ranges.push(range);
      range = null;
    }
  }
  return ranges;
}

function decorationsForSelection(doc: ProseMirrorNode, selection: Selection): DecorationSet {
  const decorations: Decoration[] = [];
  for (const range of collectStyledRanges(doc)) {
    const active = selection.empty
      ? selection.from >= range.from && selection.from <= range.to
      : selection.from < range.to && selection.to > range.from;
    if (!active) continue;
    for (const { at, side, text } of range.markers) {
      const marker = document.createElement("span");
      marker.className = "rich-marker";
      marker.textContent = text;
      marker.setAttribute("aria-hidden", "true");
      decorations.push(Decoration.widget(at, marker, { side, key: `marker-${at}-${side}-${text}` }));
    }
  }
  return DecorationSet.create(doc, decorations);
}

const markerKey = new PluginKey<DecorationSet>("rich-markers");

const MarkersExtension = Extension.create({
  name: "rich-markers",
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: markerKey,
        state: {
          init: (_, state) => decorationsForSelection(state.doc, state.selection),
          apply: (tr, old) => (tr.docChanged || tr.selectionSet ? decorationsForSelection(tr.doc, tr.selection) : old),
        },
        props: {
          decorations: (state) => markerKey.getState(state),
        },
      }),
    ];
  },
});

declare global {
  interface Window {
    webkit?: { messageHandlers: { nino: { postMessage: (text: string) => void } } };
  }
}

const root = document.getElementById("editor")!;
let done = false;

function finish(text: string) {
  if (done) return;
  done = true;
  if (window.webkit) window.webkit.messageHandlers.nino.postMessage(text);
  else console.log(text);
}

const editor = new Editor({
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
    }),
    MarkersExtension,
    NinoCode,
    TaskList,
    NinoTaskItem,
  ],
  editorProps: {
    attributes: { class: "composer", "aria-label": "Comentário" },
    handleKeyDown: (view, event) => {
      if (event.key === "Escape") {
        finish("");
        return true;
      }
      if (event.key !== "Enter") return false;
      if (event.isComposing || event.keyCode === 229) return true;
      if (event.target instanceof HTMLInputElement) {
        event.preventDefault();
        return true;
      }
      const isTaskItem = editor.isActive("taskItem");
      if (!event.shiftKey && (!isTaskItem || event.ctrlKey)) {
        event.preventDefault();
        finish(serializeEditorDoc(view.state.doc).value.trim());
        return true;
      }
      event.preventDefault();
      if (
        isTaskItem &&
        (editor.commands.splitListItem("taskItem", { checked: false }) ||
          (view.state.selection.$from.parent.content.size === 0 && editor.commands.liftListItem("taskItem")))
      ) {
        return true;
      }
      return splitBlockKeepMarks(view.state, (tr) => view.dispatch(tr.scrollIntoView()));
    },
    handleTextInput: (view, from, to, text) => {
      if (text.length !== 1) return false;
      const closer = SURROUND_CLOSE[text];
      if (!closer || from === to) return false;
      let touchesStyled = false;
      view.state.doc.nodesBetween(from, to, (node) => {
        if (node.isText && node.marks.length > 0) touchesStyled = true;
        return !touchesStyled;
      });
      if (touchesStyled) return false;
      const tr = view.state.tr.insertText(closer, to).insertText(text, from);
      tr.setSelection(TextSelection.create(tr.doc, from + text.length, to + text.length));
      view.dispatch(tr);
      return true;
    },
    handlePaste: (_, event) => {
      const text = event.clipboardData?.getData("text/plain");
      if (!text) return false;
      event.preventDefault();
      const blocks = buildTiptapContent(text);
      const only = blocks.length === 1 && blocks[0]?.type === "paragraph" ? blocks[0] : null;
      editor.commands.insertContent(only ? (only.content ?? []) : blocks);
      return true;
    },
  },
  onUpdate: ({ editor: updated }) => {
    root.dataset.empty = String(updated.isEmpty);
  },
});

root.dataset.empty = "true";
