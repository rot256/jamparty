// What a comment is about: the element (or the whole page), its selector
// and any selected text.

import { MAX_SELECTED_TEXT } from "./generated/api";

export interface Captured {
  /** The element commented on; null for a page-level comment. */
  element: Element | null;
  page: string;
  selector: string;
  selectedText: string | null;
}

function truncate(value: string, length: number): string {
  return value.length <= length ? value : `${value.slice(0, length - 1)}…`;
}

function normalizeText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function selectedTextFor(target: Node): string | null {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  try {
    if (!selection.getRangeAt(0).intersectsNode(target)) return null;
  } catch {
    return null;
  }
  const selected = normalizeText(selection.toString());
  return selected ? truncate(selected, MAX_SELECTED_TEXT) : null;
}

function uniqueIdSelector(element: Element): string | null {
  if (!element.id) return null;
  const selector = `#${CSS.escape(element.id)}`;
  return document.querySelectorAll(selector).length === 1 ? selector : null;
}

export function cssSelector(target: Element): string {
  const parts: string[] = [];
  let element: Element | null = target;
  while (element) {
    const idSelector = uniqueIdSelector(element);
    if (idSelector) {
      parts.unshift(idSelector);
      break;
    }
    let part = element.tagName.toLowerCase();
    const parent: Element | null = element.parentElement;
    if (parent) {
      const tagName = element.tagName;
      const sameTag = Array.from(parent.children).filter(
        (sibling) => sibling.tagName === tagName,
      );
      if (sameTag.length > 1) part += `:nth-of-type(${sameTag.indexOf(element) + 1})`;
    }
    parts.unshift(part);
    element = parent;
  }
  return parts.join(" > ");
}

export function captureTarget(target: Element, selector = cssSelector(target)): Captured {
  return {
    element: target,
    page: window.location.href,
    selector,
    selectedText: selectedTextFor(target),
  };
}

export function capturePageTarget(): Captured {
  return {
    element: null,
    page: window.location.href,
    selector: "",
    selectedText: selectedTextFor(document.documentElement),
  };
}

/**
 * Whether an answer's comment was made on this page (hosts differ per
 * joiner, so only the path and query count).
 */
export function samePage(page: string | undefined): boolean {
  if (page === undefined) return true;
  try {
    const url = new URL(page);
    return url.pathname === window.location.pathname && url.search === window.location.search;
  } catch {
    return false;
  }
}
