// The control panel behind the gear: the comment mode and cursor
// switches with their keys (rebindable), the name, and who is here.

import { changeSettings } from "./api";
import { PALETTE, RESERVED_KEYS } from "./config";
import type { Keys, PageSettings, Person, Saved } from "./generated/api";
import type { Ui } from "./ui";

type Action = keyof Keys;

const ACTION_NAMES: Record<Action, string> = {
  commentMode: "comment mode",
  showCursor: "show my cursor",
};

/** A key as it is bound: single characters in lower case. */
export function normalizeKey(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

/** A binding as its chip shows it. */
function keyLabel(key: string): string {
  return key.length === 1 ? key.toUpperCase() : key;
}

/** Whether a key press belongs to a text field (the page's or ours). */
export function isEditing(event: Event): boolean {
  return event.composedPath().some(
    (node) => node instanceof HTMLElement &&
      (node.isContentEditable || node.matches("input, textarea, select")),
  );
}

/** Up to two initials: `alice` → `A`, `bob.smith` → `BS`. */
export function initials(name: string): string {
  const parts = name.split("@")[0]?.split(/[._+-]+/u).filter((part) => part !== "") ?? [];
  const letters = parts.slice(0, 2).map((part) => part.charAt(0).toUpperCase()).join("");
  return letters === "" ? "?" : letters;
}

export function colourOf(person: Person): string {
  return PALETTE[person.colour % PALETTE.length] ?? "#555";
}

export class Panel {
  private settings: PageSettings | null = null;
  private listening: Action | null = null;
  private you: string | null = null;
  /** Called when the panel opens (the comment list closes then). */
  onOpen: () => void = () => undefined;
  /** Called with the settings whenever they change. */
  onSettings: (settings: PageSettings) => void = () => undefined;
  /** Whether a saved answer is newer than what the page has applied. */
  fresh: (saved: Saved) => boolean = () => true;

  constructor(private readonly ui: Ui) {
    ui.gear.addEventListener("click", () => { this.toggle(); });
    ui.commentMode.addEventListener("click", () => { this.flip("commentMode"); });
    ui.showCursor.addEventListener("click", () => { this.flip("showCursor"); });
    for (const [action, chip] of this.chips()) {
      chip.addEventListener("click", () => { this.listen(action); });
    }
    ui.nameField.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        this.saveName();
      }
    });
    ui.nameField.addEventListener("change", () => { this.saveName(); });
    // Keys are handled before the page sees them, and only outside text fields.
    document.addEventListener("keydown", (event) => { this.onKey(event); }, true);
    // A press anywhere outside the open panel closes it (the gear toggles
    // it itself); the press still does what it does.
    document.addEventListener("pointerdown", (event) => {
      const path = event.composedPath();
      if (this.isOpen && !path.includes(ui.panel) && !path.includes(ui.gear)) this.close();
    }, true);
  }

  private chips(): [Action, HTMLButtonElement][] {
    return [["commentMode", this.ui.commentModeKey], ["showCursor", this.ui.showCursorKey]];
  }

  get isOpen(): boolean {
    return !this.ui.panel.hidden;
  }

  get commentMode(): boolean {
    return this.settings?.commentMode !== "off";
  }

  toggle(): void {
    if (this.isOpen) this.close();
    else this.open();
  }

  open(): void {
    this.onOpen();
    this.ui.panel.hidden = false;
    this.ui.gear.setAttribute("aria-expanded", "true");
  }

  close(): void {
    this.stopListening();
    this.ui.panel.hidden = true;
    this.ui.gear.setAttribute("aria-expanded", "false");
    this.ui.panelMessage.textContent = "";
  }

  private message(text: string): void {
    this.ui.panelMessage.textContent = text;
  }

  /** The settings from the joiner (status or a settings event). */
  show(settings: PageSettings): void {
    this.settings = settings;
    const { ui } = this;
    ui.commentMode.setAttribute("aria-checked", String(settings.commentMode === "on"));
    ui.showCursor.setAttribute("aria-checked", String(settings.showCursor === "on"));
    for (const [action, chip] of this.chips()) {
      if (this.listening !== action) chip.textContent = keyLabel(settings.keys[action]);
    }
    if (ui.shadow.activeElement !== ui.nameField) ui.nameField.value = settings.name;
    this.onSettings(settings);
  }

  private async change(change: Parameters<typeof changeSettings>[0]): Promise<void> {
    try {
      const saved = await changeSettings(change);
      // A `settings` event may have brought newer settings meanwhile.
      if (this.fresh(saved)) this.show(saved);
    } catch (error) {
      this.message(error instanceof Error ? error.message : "could not save");
    }
  }

  private flip(action: Action): void {
    const settings = this.settings;
    if (settings === null) return;
    const value = settings[action] === "on" ? "off" : "on";
    this.message("");
    void this.change(action === "commentMode" ? { commentMode: value } : { showCursor: value });
  }

  private saveName(): void {
    const name = this.ui.nameField.value.trim();
    if (this.settings === null || name === this.settings.name) return;
    this.message("");
    void this.change({ name });
  }

  private listen(action: Action): void {
    this.stopListening();
    this.listening = action;
    const chip = action === "commentMode" ? this.ui.commentModeKey : this.ui.showCursorKey;
    chip.dataset.listening = "true";
    chip.textContent = "press a key…";
    this.message("");
  }

  private stopListening(): void {
    this.listening = null;
    for (const [action, chip] of this.chips()) {
      chip.dataset.listening = "false";
      if (this.settings !== null) chip.textContent = keyLabel(this.settings.keys[action]);
    }
  }

  /** The next key press while a chip listens becomes its binding. */
  private rebind(action: Action, event: KeyboardEvent): void {
    event.preventDefault();
    event.stopImmediatePropagation();
    const settings = this.settings;
    if (event.key === "Escape" || settings === null) {
      this.stopListening();
      return;
    }
    if (["Shift", "Control", "Alt", "Meta", "CapsLock"].includes(event.key)) return;
    const key = normalizeKey(event.key);
    const other: Action = action === "commentMode" ? "showCursor" : "commentMode";
    if (RESERVED_KEYS.includes(key) || key === " ") {
      this.stopListening();
      this.message(`${keyLabel(key)} is used by jampot`);
      return;
    }
    if (normalizeKey(settings.keys[other]) === key) {
      this.stopListening();
      this.message(`${keyLabel(key)} is taken by ${ACTION_NAMES[other]}`);
      return;
    }
    const keys = { ...settings.keys, [action]: key };
    this.stopListening();
    void this.change({ keys });
  }

  private onKey(event: KeyboardEvent): void {
    if (this.listening !== null) {
      this.rebind(this.listening, event);
      return;
    }
    const settings = this.settings;
    if (
      settings === null || event.defaultPrevented || event.isComposing ||
      event.metaKey || event.ctrlKey || event.altKey || isEditing(event)
    ) return;
    const key = normalizeKey(event.key);
    for (const action of ["commentMode", "showCursor"] as const) {
      if (normalizeKey(settings.keys[action]) === key) {
        event.preventDefault();
        event.stopImmediatePropagation();
        this.flip(action);
        return;
      }
    }
  }

  /** Who is here: the count and an avatar each, the joiner's own outlined. */
  showPeople(here: readonly Person[], you: string | null): void {
    this.you = you;
    const { hereCount, avatars } = this.ui;
    hereCount.textContent = `${here.length} here`;
    const ordered = [
      ...here.filter((person) => person.key === this.you),
      ...here.filter((person) => person.key !== this.you),
    ];
    avatars.replaceChildren(...ordered.map((person) => {
      const item = document.createElement("li");
      item.className = "avatar";
      const mine = person.key === this.you;
      item.dataset.you = String(mine);
      item.dataset.key = person.key;
      item.title = mine ? `${person.name} (you)` : person.name;
      item.textContent = initials(person.name);
      if (!mine) item.style.background = colourOf(person);
      return item;
    }));
  }
}
