import { expect, test } from "bun:test";
import { WindowCloseReceipts } from "./window-close-receipts.ts";

const WINDOW = { id: 7, incarnation: 300n };
const STATE = {
  bounds: { x: 0, y: 0, width: 640, height: 480 },
  focused: false,
  maximized: false,
  full_screen: false,
};

test("dispatched pending close retains a newer sequence across old Veto ack", () => {
  const receipts = new WindowCloseReceipts();
  receipts.observe({
    variant: "Created",
    window: WINDOW,
    state: STATE,
    pending_close: 7n,
  });
  const pending = () => receipts.pending().map(({ closeSeq }) => closeSeq);
  expect(pending()).toEqual([7n]);
  receipts.acceptHostCloseReply({ ...WINDOW, id: 8 }, 7n);
  receipts.acceptHostCloseReply({ ...WINDOW, incarnation: 301n }, 7n);
  receipts.acceptHostCloseReply(WINDOW, 6n);
  expect(pending()).toEqual([7n]);
  receipts.acceptHostCloseReply(WINDOW, 7n);
  expect(pending()).toEqual([]);

  receipts.observe({
    variant: "CloseRequested",
    window: WINDOW,
    close_seq: 8n,
  });
  receipts.acceptHostCloseReply(WINDOW, 7n);
  expect(pending()).toEqual([8n]);
  receipts.acceptHostCloseReply(WINDOW, 8n);
  expect(pending()).toEqual([]);
  receipts.observe({ variant: "Destroyed", window: WINDOW });
  expect(receipts.pending()).toEqual([]);
});

test("Veto ack arriving before retained Created/CloseRequested dispatch suppresses only older sequence", () => {
  const receipts = new WindowCloseReceipts();
  const pending = () => receipts.pending().map(({ closeSeq }) => closeSeq);
  // The authentic source already applied Created(pending7) but dispatch was
  // blocked on an unrelated sync CALL; the accepted Veto reply came first.
  receipts.acceptHostCloseReply(WINDOW, 7n);
  receipts.observe({
    variant: "Created",
    window: WINDOW,
    state: STATE,
    pending_close: 7n,
  });
  expect(pending()).toEqual([]);
  receipts.observe({
    variant: "CloseRequested",
    window: WINDOW,
    close_seq: 7n,
  });
  expect(pending()).toEqual([]);
  receipts.observe({
    variant: "CloseRequested",
    window: WINDOW,
    close_seq: 8n,
  });
  expect(pending()).toEqual([8n]);
  receipts.acceptHostCloseReply(WINDOW, 8n);
  expect(pending()).toEqual([]);
  receipts.observe({
    variant: "CloseRequested",
    window: WINDOW,
    close_seq: 7n,
  });
  expect(pending()).toEqual([]);
});
