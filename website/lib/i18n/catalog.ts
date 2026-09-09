import en from "../../locales/en/messages.json";
export type MessageKey = keyof typeof en;
export type Messages = Record<MessageKey, string>;
const sourceKeys = new Map(Object.entries(en).map(([key, text]) => [text, key as MessageKey]));
export function translateSource(text: string, messages: Messages): string {
  const key = sourceKeys.get(text);
  return key ? messages[key] : text;
}
