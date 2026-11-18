export type ComposerPromptSegment =
  | {
      type: "text";
      text: string;
    }
  | {
      type: "mention";
      path: string;
      source: string;
    }
  | {
      type: "skill";
      name: string;
      source: string;
    }
  | {
      type: "model";
      instanceId: string;
      model: string;
      source: string;
    }
  | {
      type: "citation";
      citation: unknown;
      source: string;
    }
  | {
      type: "context-reference";
      kind: string;
      contextId: string;
      label: string;
      source: string;
    };
export function splitPromptIntoComposerSegments(prompt: string): ComposerPromptSegment[] {
  return prompt ? [{ type: "text", text: prompt }] : [];
}
