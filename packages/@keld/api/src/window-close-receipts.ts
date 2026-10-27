/**
 * Only authenticated *dispatched* close facts qualify for late-import catch-up.
 * The WorkerLink may apply later Host facts ahead of event dispatch while a
 * synchronous caller is parked. A mirror snapshot alone has no proof that the
 * CloseRequested callback reached this role before earlier Bounds callbacks.
 */
import type { WindowEvent, WindowRef } from "./window-codec.ts";

export interface DeliveredWindowClose {
  readonly window: Readonly<WindowRef>;
  readonly closeSeq: bigint;
}

export class WindowCloseReceipts {
  readonly #records = new Map<
    number,
    {
      readonly ref: Readonly<WindowRef>;
      pending: bigint | null;
      /** Highest authenticated settled close; bounded to one scalar per Host id. */
      ackedThrough: bigint | null;
    }
  >();

  observe(fact: WindowEvent): void {
    const { id, incarnation } = fact.window;
    if (fact.variant === "Created") {
      // WorkerLink can apply a Host event before dispatching it while a CALL
      // is parked. A correlated Veto reply can therefore arrive before this
      // Created(pending) reaches the subscriber. Carry the one-scalar Host-ack
      // watermark across that delayed dispatch; never resurrect an old close.
      const previous = this.#records.get(id);
      const ackedThrough =
        previous?.ref.incarnation === incarnation
          ? previous.ackedThrough
          : null;
      const pending =
        ackedThrough !== null &&
        fact.pending_close !== null &&
        fact.pending_close <= ackedThrough
          ? null
          : fact.pending_close;
      this.#records.set(id, {
        ref: Object.freeze({ id, incarnation }),
        pending,
        ackedThrough,
      });
      return;
    }
    const earlier = this.#records.get(id);
    if (!earlier || earlier.ref.incarnation !== incarnation) return;
    if (fact.variant === "Destroyed") this.#records.delete(id);
    if (fact.variant === "CloseRequested") {
      // A later *dispatched* fact may be older than an already acknowledged
      // Host Veto because the transport may apply ahead of its user cursor.
      if (
        earlier.ackedThrough !== null &&
        fact.close_seq <= earlier.ackedThrough
      )
        return;
      earlier.pending = fact.close_seq;
    }
  }

  /** Only after the exact Host CloseReply ack; preserve later dispatched seq. */
  acceptHostCloseReply(window: WindowRef, closeSeq: bigint): void {
    const previous = this.#records.get(window.id);
    if (previous && previous.ref.incarnation !== window.incarnation) return;
    if (!previous) {
      // Source may already have applied Created while its user dispatch is
      // still queued. No unbounded per-close tombstones: one watermark only.
      this.#records.set(window.id, {
        ref: Object.freeze({ id: window.id, incarnation: window.incarnation }),
        pending: null,
        ackedThrough: closeSeq,
      });
      return;
    }
    if (previous.ackedThrough === null || closeSeq > previous.ackedThrough)
      previous.ackedThrough = closeSeq;
    if (previous.pending !== null && previous.pending <= previous.ackedThrough)
      previous.pending = null;
  }

  pending(): readonly DeliveredWindowClose[] {
    const result: DeliveredWindowClose[] = [];
    for (const record of this.#records.values()) {
      if (record.pending !== null) {
        result.push(
          Object.freeze({ window: record.ref, closeSeq: record.pending }),
        );
      }
    }
    return Object.freeze(result);
  }
}
