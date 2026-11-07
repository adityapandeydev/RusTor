import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { Artwork } from "./Artwork";

afterEach(cleanup);
const png = "data:image/png;base64,aGVsbG8=";
it("loads native artwork by identity and reveals it after decoding", async () => {
  const load = vi.fn().mockResolvedValue(png);
  const { container } = render(<Artwork infoHash={"a".repeat(40)} load={load} />);
  await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
  expect(load).toHaveBeenCalledWith("a".repeat(40));
  const image = container.querySelector("img")!;
  expect(image.alt).toBe("");
  expect(image.className).toBe("");
  fireEvent.load(image);
  expect(image.className).toBe("artwork-loaded");
  fireEvent.error(image);
  expect(container.querySelector("img")).toBeNull();
});
it("skips missing artwork and rejects remote or executable image sources", async () => {
  const load = vi.fn().mockResolvedValue("https://remote.example/a.jpg");
  const { rerender, container } = render(<Artwork infoHash="a" available={false} load={load} />);
  expect(load).not.toHaveBeenCalled();
  rerender(<Artwork infoHash="a" load={load} />);
  await waitFor(() => expect(load).toHaveBeenCalledOnce());
  expect(container.querySelector("img")).toBeNull();
});
it("ignores a late result after the row changes identity", async () => {
  let resolve!: (value: string) => void;
  const load = vi.fn().mockImplementationOnce(() => new Promise<string>((done) => { resolve = done; })).mockResolvedValue(null);
  const { rerender, container } = render(<Artwork infoHash="first" load={load} />);
  rerender(<Artwork infoHash="second" load={load} />);
  resolve(png);
  await waitFor(() => expect(load).toHaveBeenCalledTimes(2));
  expect(container.querySelector("img")).toBeNull();
});
