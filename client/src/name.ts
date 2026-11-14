// The first-join name prompt: "Join the potluck." over a dimmed page,
// until the joiner has a name chosen (or one given with --name).

import type { PageSettings } from "./generated/api";
import { type ChangeSettings, failure } from "./panel";
import type { Ui } from "./ui";

export class NamePrompt {
  constructor(
    private readonly ui: Ui,
    private readonly changeSettings: ChangeSettings,
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
    // The name taken comes as a settings message, which closes the prompt.
    const failed = failure(await this.changeSettings({ name: nameInput.value.trim() }));
    nameJoin.disabled = false;
    if (failed !== null) {
      nameError.textContent = failed;
      nameInput.focus();
    }
  }
}
