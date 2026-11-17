import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { Link } from "./Link";
import { Breadcrumbs } from "./Breadcrumbs";

afterEach(cleanup);

// ---------------------------------------------------------------------------
// P01-P03: the action primitives share one grammar.
//
// `interactionStates.test.tsx` owns the state matrix -- rest, disabled, busy,
// selected, focus. This file owns the CONTRACT the three actions agree on:
// which axis each prop names, which words each axis uses, and what an anchor
// has to do for a click on it to reach anything at all.
// ---------------------------------------------------------------------------

describe("Button and IconButton speak one vocabulary", () => {
  it("keeps hierarchy and geometry on separate props", () => {
    // A `pill` variant used to sit beside the `pill` shape, so a "pill button"
    // named two things at once and neither said which hierarchy it carried.
    render(
      <>
        <Button shape="pill" variant="primary">
          Round and primary
        </Button>
        <Button shape="default" variant="primary">
          Square and primary
        </Button>
      </>,
    );
    const round = screen.getByRole("button", { name: "Round and primary" });
    const square = screen.getByRole("button", { name: "Square and primary" });
    expect(round.className).toContain("rounded-full");
    expect(square.className).toContain("rounded-md");
    // Same hierarchy, so the same fill -- shape changed nothing else.
    expect(round.className).toContain("bg-accent");
    expect(square.className).toContain("bg-accent");
  });

  it("paints destructive ink with the danger ink role, never a surface token", () => {
    render(<Button variant="destructive">Delete</Button>);
    const button = screen.getByRole("button", { name: "Delete" });
    expect(button.className).toContain("text-danger-ink");
    // `text-card` contrasts against danger in both themes today by coincidence
    // of the palette, and would stop the day either value moves.
    expect(button.className).not.toContain("text-card");
  });

  it("states busy the same way in both action primitives", () => {
    render(
      <>
        <Button loading>Saving</Button>
        <Button>Idle</Button>
        <IconButton aria-label="Refreshing" loading />
        <IconButton aria-label="Idle icon" />
      </>,
    );
    expect(screen.getByRole("button", { name: "Saving" })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "Refreshing" })).toHaveAttribute("aria-busy", "true");
    // Absent, not "false": a control that is not working has nothing to say
    // about working, and the two primitives disagreed about that.
    expect(screen.getByRole("button", { name: "Idle" })).not.toHaveAttribute("aria-busy");
    expect(screen.getByRole("button", { name: "Idle icon" })).not.toHaveAttribute("aria-busy");
  });

  it("gives an icon action its hierarchy on `variant`, leaving `tone` to status", () => {
    render(
      <>
        <IconButton aria-label="Delete row" variant="danger" />
        <IconButton aria-label="Send" variant="primary" />
        <IconButton aria-label="More" />
      </>,
    );
    expect(screen.getByRole("button", { name: "Delete row" }).className).toContain("hover:text-danger");
    expect(screen.getByRole("button", { name: "Send" }).className).toContain("bg-accent");
    expect(screen.getByRole("button", { name: "More" }).className).toContain("text-muted");
  });

  it("declares the press transition once, in `press`, and never beside it", () => {
    render(<IconButton aria-label="Close" />);
    const button = screen.getByRole("button", { name: "Close" });
    expect(button.className).toContain("press");
    // `transition-colors` narrowed the property list to colour alone, dropping
    // the transform the press affordance rides on.
    expect(button.className).not.toContain("transition-colors");
  });
});

describe("hover and press never cost an action its contrast", () => {
  it("fills a hovered or pressed accent action with a fill token, never the accent-as-text colour", () => {
    // `accent-hover` is the accent drawn as TEXT on a light surface. As a fill it
    // darkened the button under its own dark ink (under 3:1), the moment the
    // pointer arrived.
    render(
      <>
        <Button variant="primary">Save</Button>
        <IconButton aria-label="Go" variant="primary">
          <span />
        </IconButton>
      </>,
    );
    for (const control of [screen.getByRole("button", { name: "Save" }), screen.getByRole("button", { name: "Go" })]) {
      expect(control.className).toContain("hover:bg-accent-fill-hover");
      expect(control.className).toContain("active:bg-accent-fill-active");
      expect(control.className).not.toContain("hover:bg-accent-hover");
    }
  });

  it("gives every quiet action a distinct pressed fill beside its hover fill", () => {
    render(
      <>
        <Button variant="secondary">Secondary</Button>
        <Button variant="ghost">Ghost</Button>
        <IconButton aria-label="Icon">
          <span />
        </IconButton>
      </>,
    );
    for (const name of ["Secondary", "Ghost", "Icon"]) {
      const control = screen.getByRole("button", { name });
      expect(control.className).toContain("hover:bg-hover");
      expect(control.className).toContain("active:bg-active");
    }
  });

  it("keeps a selected action's fill under the pointer and under a press", () => {
    // The hover fill is lighter than the selected one, so a selected action that
    // took it back when hovered would appear to deselect itself.
    render(
      <>
        <Button selected variant="ghost">
          On
        </Button>
        <IconButton aria-label="Also on" selected>
          <span />
        </IconButton>
      </>,
    );
    for (const name of ["On", "Also on"]) {
      const control = screen.getByRole("button", { name });
      expect(control.className).toContain("bg-active");
      expect(control.className).toContain("hover:bg-active");
      expect(control.className).not.toContain("hover:bg-hover");
    }
  });
});

describe("Link", () => {
  // The four schemes `tauri_plugin_opener` forwards to the operating system,
  // and it forwards them ONLY for `target="_blank"`. Without the target the
  // click is a navigation, the navigation boundary refuses it, and nothing
  // happens -- no browser, no error, no trace.
  const leaving = [
    ["https://xfalcon.ai/docs", "opens in the browser"],
    ["http://example.test/", "opens in the browser"],
    ["mailto:support@xfalcon.ai", "opens the mail app"],
    ["tel:+15550100", "opens the phone app"],
  ] as const;

  it.each(leaving)("hands %s to the system and says where it goes", (href, destination) => {
    render(<Link href={href}>Contact</Link>);
    const link = screen.getByRole("link", { name: `Contact, ${destination}` });
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(link).toHaveAttribute("href", href);
  });

  it.each(["/projects", "#section", "file:///tmp/notes.txt", "ftp://host/file"])(
    "leaves %s inside the app, with no target and no destination note",
    (href) => {
      render(<Link href={href}>Somewhere</Link>);
      const link = screen.getByRole("link", { name: "Somewhere" });
      expect(link).not.toHaveAttribute("target");
      expect(link).not.toHaveAttribute("rel");
    },
  );

  it("does not announce the destination twice when the caller names the link", () => {
    render(
      <Link aria-label="Read the docs in your browser" href="https://xfalcon.ai/docs">
        Docs
      </Link>,
    );
    expect(screen.getByRole("link", { name: "Read the docs in your browser" })).toBeInTheDocument();
  });

  it("honours an explicit external flag over the scheme", () => {
    render(
      <Link external={false} href="https://xfalcon.ai/docs">
        Handled here
      </Link>,
    );
    expect(screen.getByRole("link", { name: "Handled here" })).not.toHaveAttribute("target");
  });

  it("underlines body links on hover and leaves navigation links unadorned", () => {
    render(
      <>
        <Link href="/a">Body</Link>
        <Link href="/b" variant="nav">
          Trail
        </Link>
      </>,
    );
    expect(screen.getByRole("link", { name: "Body" }).className).toContain("hover:underline");
    expect(screen.getByRole("link", { name: "Trail" }).className).not.toContain("hover:underline");
  });
});

describe("Breadcrumbs", () => {
  it("gives both kinds of crumb the same resting and hover treatment", () => {
    render(
      <Breadcrumbs
        items={[{ href: "/projects", label: "Projects" }, { label: "Reindex", onClick: () => {} }, { label: "Here" }]}
      />,
    );
    const addressed = screen.getByRole("link", { name: "Projects" });
    const acting = screen.getByRole("button", { name: "Reindex" });
    for (const utility of ["text-muted", "hover:text-foreground", "rounded-sm"]) {
      expect(addressed.className).toContain(utility);
      expect(acting.className).toContain(utility);
    }
    // The last crumb is the place you are, not a control.
    expect(screen.getByText("Here")).toHaveAttribute("aria-current", "page");
  });
});
