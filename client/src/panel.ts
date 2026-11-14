// The control panel behind the gear: the comment mode and cursor
// switches with their keys (rebindable), the name, and who is here.

import { PALETTE } from "./config";
import { MODIFIER_KEYS, PAGE_KEYS, RESERVED_KEYS } from "./generated/api";
import type {
  Corner,
  Keys,
  PageSettings,
  Person,
  SettingsChange,
  SettingsOutcome,
} from "./generated/api";
import type { Ui } from "./ui";

type Action = keyof Keys;

const CORNERS: readonly Corner[] = ["bottom-right", "bottom-left", "top-right", "top-left"];

function isCorner(value: string | undefined): value is Corner {
  return CORNERS.some((corner) => corner === value);
}

const ACTION_NAMES: Record<Action, string> = {
  commentMode: "comment mode",
  showCursor: "show my cursor",
};

/** A key as it is bound: single characters in lower case. */
function normalizeKey(key: string): string {
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
function initials(name: string): string {
  const parts = name.split("@")[0]?.split(/[._+-]+/u).filter((part) => part !== "") ?? [];
  const letters = parts.slice(0, 2).map((part) => part.charAt(0).toUpperCase()).join("");
  return letters === "" ? "?" : letters;
}

/** Changes the joiner's settings; what became of the change. */
export type ChangeSettings = (change: SettingsChange) => Promise<SettingsOutcome>;

/** Why a change did not go through, or null when it did. */
export function failure(outcome: SettingsOutcome): string | null {
  return outcome === "saved" ? null : outcome.refused.error;
}

export function colourOf(person: Person): string {
  return PALETTE[person.colour % PALETTE.length] ?? "#555";
}

export class Panel {
  private settings: PageSettings | null = null;
  private listening: Action | null = null;
  /** Called when the panel opens (the comment list closes then). */
  onOpen: () => void = () => undefined;
  /** Called with the settings whenever they change. */
  onSettings: (settings: PageSettings) => void = () => undefined;

  constructor(
    private readonly ui: Ui,
    private readonly changeSettings: ChangeSettings,
  ) {
    ui.gear.addEventListener("click", () => { this.toggle(); });
    ui.commentMode.addEventListener("click", () => { this.flip("commentMode"); });
    ui.showCursor.addEventListener("click", () => { this.flip("showCursor"); });
    for (const button of ui.corners) {
      button.addEventListener("click", () => {
        const corner = button.dataset.corner;
        if (!isCorner(corner) || this.settings?.corner === corner) return;
        this.message("");
        void this.change({ corner });
      });
    }
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

  /** The settings from the joiner (its hello or a settings message). */
  show(settings: PageSettings): void {
    this.settings = settings;
    const { ui } = this;
    ui.commentMode.setAttribute("aria-checked", String(settings.commentMode === "on"));
    ui.showCursor.setAttribute("aria-checked", String(settings.showCursor === "on"));
    for (const [action, chip] of this.chips()) {
      if (this.listening !== action) chip.textContent = keyLabel(settings.keys[action]);
    }
    if (ui.shadow.activeElement !== ui.nameField) ui.nameField.value = settings.name;
    for (const button of ui.corners) {
      button.setAttribute("aria-pressed", String(button.dataset.corner === settings.corner));
    }
    // The styles place the pill, gear and panels by the host's corner.
    ui.host.dataset.corner = settings.corner;
    this.onSettings(settings);
  }

  /**
   * Asks the joiner for a change. What it took comes as a settings
   * message, in order with everyone else's; only a failure is shown here.
   */
  private async change(change: SettingsChange): Promise<void> {
    const failed = failure(await this.changeSettings(change));
    if (failed !== null) this.message(failed);
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
    // A modifier goes with the key after it: wait for that.
    if (MODIFIER_KEYS.includes(event.key)) return;
    const key = normalizeKey(event.key);
    const other: Action = action === "commentMode" ? "showCursor" : "commentMode";
    if (RESERVED_KEYS.includes(key)) {
      this.stopListening();
      this.message(`${keyLabel(key)} is used by jampot`);
      return;
    }
    if (PAGE_KEYS.includes(key)) {
      this.stopListening();
      this.message(`${key === " " ? "Space" : key} is the page's`);
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
        // A held key flips once, not once per auto-repeat.
        if (!event.repeat) this.flip(action);
        return;
      }
    }
  }

  /** Who is here: the count and an avatar each, the joiner's own outlined. */
  showPeople(here: readonly Person[], you: string | null): void {
    const { hereCount, avatars } = this.ui;
    hereCount.textContent = `${here.length} here`;
    const ordered = [
      ...here.filter((person) => person.key === you),
      ...here.filter((person) => person.key !== you),
    ];
    avatars.replaceChildren(...ordered.map((person) => {
      const item = document.createElement("li");
      item.className = "avatar";
      const mine = person.key === you;
      item.dataset.you = String(mine);
      item.dataset.key = person.key;
      item.title = mine ? `${person.name} (you)` : person.name;
      item.textContent = initials(person.name);
      if (!mine) item.style.background = colourOf(person);
      return item;
    }));
  }
}
