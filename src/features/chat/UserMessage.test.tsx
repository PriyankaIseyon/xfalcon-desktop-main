import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { UserMessage } from "./UserMessage";

describe("UserMessage", () => {
  afterEach(cleanup);

  it("names the files sent with the turn under its text", () => {
    render(
      <UserMessage
        text="How many rows?"
        attachments={[
          { attachment_id: "a-1", file_name: "schema_consistent.csv", mime_type: "text/csv" },
          { attachment_id: "a-2", file_name: "headers.xlsx", mime_type: "" },
        ]}
      />,
    );

    expect(screen.getByText("How many rows?")).toBeInTheDocument();
    const list = screen.getByRole("list", { name: "Attached files" });
    expect(list).toHaveTextContent("schema_consistent.csv");
    expect(list).toHaveTextContent("headers.xlsx");
  });

  it("shows no file list for a turn that carried none", () => {
    render(<UserMessage text="Just a question" />);

    expect(screen.queryByRole("list", { name: "Attached files" })).not.toBeInTheDocument();
  });
});
