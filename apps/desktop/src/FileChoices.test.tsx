import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { FileChoices } from "./FileChoices";

afterEach(cleanup);
it("keeps selections across pages and search while rendering at most 100 checkboxes", async () => {
  const files = Array.from({ length: 205 }, (_, index) => ({
    index,
    path: `folder/file-${index}.bin`,
    length: 1024,
    selected: true,
  }));
  function Harness() {
    const [selected, setSelected] = useState(
      new Set(files.map((file) => file.index)),
    );
    return (
      <FileChoices
        files={files}
        selected={selected}
        onChange={setSelected}
        disabled={false}
      />
    );
  }
  render(<Harness />);
  expect(screen.getAllByRole("checkbox")).toHaveLength(100);
  await userEvent.click(
    screen.getByRole("checkbox", { name: "folder/file-0.bin" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Next files" }));
  expect(
    screen.getByRole("checkbox", { name: "folder/file-100.bin" }),
  ).toHaveProperty("checked", true);
  await userEvent.type(
    screen.getByRole("textbox", { name: "Find files in torrent" }),
    "file-0.bin",
  );
  expect(screen.getAllByRole("checkbox")).toHaveLength(1);
  expect(
    screen.getByRole("checkbox", { name: "folder/file-0.bin" }),
  ).toHaveProperty("checked", false);
  await userEvent.click(
    screen.getByRole("button", { name: "Clear selection" }),
  );
  expect(screen.getByText(/0 of 205 selected/)).toBeDefined();
  await userEvent.click(
    screen.getByRole("button", { name: "Select all files" }),
  );
  expect(screen.getByText(/205 of 205 selected/)).toBeDefined();
});
