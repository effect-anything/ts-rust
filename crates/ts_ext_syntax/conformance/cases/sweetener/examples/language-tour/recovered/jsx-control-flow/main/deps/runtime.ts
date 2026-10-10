export type Properties = Readonly<Record<string, unknown>> | null;

export interface Element {
  readonly tag: string;
  readonly props: Properties;
  readonly children: readonly unknown[];
}

/** Classic JSX factory, so the fixture needs nothing outside its own folder. */
export function h(
  tag: string,
  props: Properties,
  ...children: readonly unknown[]
): Element {
  return { tag, props, children };
}

export const Fragment = "fragment";

export function render(node: unknown): string {
  if (node === null || node === undefined || node === false) return "";
  if (Array.isArray(node)) return node.map(render).join("");
  if (typeof node !== "object") return String(node);
  const element = node as Element;
  return `<${element.tag}>${element.children.map(render).join("")}</${element.tag}>`;
}

type JsxElement = Element;

declare global {
  // JSX types are declared through this namespace and no other way, so the
  // rule against namespaces cannot apply here.
  // eslint-disable-next-line @typescript-eslint/no-namespace
  namespace JSX {
    type Element = JsxElement;
    type ElementType = string;
    interface IntrinsicElements {
      readonly [tag: string]: Readonly<Record<string, unknown>>;
    }
  }
}
