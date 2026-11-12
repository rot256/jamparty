// The first-join name prompt: "Join the potluck." over a dimmed page,
// until the joiner has a name saved (or one given with --name).

import { changeSettings } from "./api";
import type { PageSettings } from "./generated/api";
import type { Ui } from "./ui";

export class NamePrompt {
  constructor(
    private readonly ui: Ui,
    private readonly saved: (settings: PageSettings) => void,
  ) {
    ui.nameForm.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.join();
    });
    // A name is needed: Escape does not dismiss it.
    ui.nameDialog.addEventListener("cancel", (event) => { event.preventDefault(); });
  }

  /** Shows or hides the prompt as the settings say. */
  show(settings: PageSettings): void {
    const { nameDialog, nameInput, nameError } = this.ui;
    if (!settings.askName) {
      if (nameDialog.open) nameDialog.close();
      return;
    }
    if (nameDialog.open || this.ui.dialog.open) return;
    nameInput.value = settings.name;
    nameError.textContent = "";
    nameDialog.showModal();
    nameInput.focus();
    nameInput.select();
  }

  private async join(): Promise<void> {
    const { nameInput, nameError, nameJoin } = this.ui;
    nameJoin.disabled = true;
    try {
      this.saved(await changeSettings({ name: nameInput.value.trim() }));
    } catch (error) {
      nameError.textContent = error instanceof Error ? error.message : "could not save the name";
      nameInput.focus();
    } finally {
      nameJoin.disabled = false;
    }
  }
}
