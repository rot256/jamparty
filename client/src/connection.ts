// The connection state the page shows: its socket to the joiner, the
// joiner's link to the share and the share's status line.

import type { Link } from "./generated/api";

export type Colour = "green" | "amber" | "grey";

export interface ConnectionView {
  colour: Colour;
  /** What the state means, for people. */
  text: string;
  /** Set when the joiner cannot serve this page; nothing is connected then. */
  incompatible: string | null;
}

export class Connection {
  private socketOpen = false;
  private link: Link | null = null;
  private incompatible: string | null = null;
  private colour: Colour = "amber";

  constructor(
    /** The state changed; `recovered` when it turned green again. */
    private readonly changed: (view: ConnectionView, recovered: boolean) => void,
  ) {}

  get view(): ConnectionView {
    let colour: Colour = "green";
    let text = "connected";
    if (this.incompatible !== null) {
      colour = "grey";
      text = this.incompatible;
    } else if (!this.socketOpen) {
      colour = "amber";
      text = "reconnecting to jampot join…";
    } else if (this.link?.connected !== true) {
      colour = "grey";
      text = "share unreachable";
    } else if (this.link.status !== null) {
      colour = "grey";
      text = this.link.status;
    }
    return { colour, text, incompatible: this.incompatible };
  }

  setOpen(open: boolean): void {
    this.socketOpen = open;
    this.render();
  }

  setLink(link: Link): void {
    this.link = link;
    this.render();
  }

  refuse(reason: string): void {
    this.incompatible = reason;
    this.render();
  }

  private render(): void {
    const view = this.view;
    const recovered = view.colour === "green" && this.colour !== "green";
    this.colour = view.colour;
    this.changed(view, recovered);
  }
}
