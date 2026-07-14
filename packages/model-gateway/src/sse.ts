import type { ReadableByteStreamLike } from "./types.ts";

export class SseProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SseProtocolError";
  }
}

export async function* parseServerSentEvents(
  body: ReadableByteStreamLike,
  maxEventCharacters = 4 * 1024 * 1024,
): AsyncGenerator<string> {
  const reader = body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let buffer = "";
  let dataLines: string[] = [];
  let completed = false;

  const flushEvent = (): string | undefined => {
    if (dataLines.length === 0) return undefined;
    const data = dataLines.join("\n");
    dataLines = [];
    return data;
  };

  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) {
        buffer += decoder.decode();
        completed = true;
        break;
      }
      if (chunk.value) buffer += decoder.decode(chunk.value, { stream: true });
      if (buffer.length > maxEventCharacters) {
        throw new SseProtocolError("SSE event exceeded the configured size limit");
      }

      let newline = buffer.indexOf("\n");
      while (newline >= 0) {
        let line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        if (line.endsWith("\r")) line = line.slice(0, -1);
        if (line === "") {
          const event = flushEvent();
          if (event === "[DONE]") return;
          if (event !== undefined) yield event;
        } else if (line.startsWith("data:")) {
          dataLines.push(line.slice(5).replace(/^ /, ""));
        }
        newline = buffer.indexOf("\n");
      }
    }

    if (buffer) {
      const line = buffer.endsWith("\r") ? buffer.slice(0, -1) : buffer;
      if (line.startsWith("data:")) dataLines.push(line.slice(5).replace(/^ /, ""));
    }
    const event = flushEvent();
    if (event !== undefined && event !== "[DONE]") yield event;
  } catch (error) {
    if (error instanceof SseProtocolError) throw error;
    throw new SseProtocolError(
      error instanceof Error ? error.message : "Failed to decode SSE stream",
    );
  } finally {
    if (!completed) {
      try {
        await reader.cancel("SSE consumer stopped");
      } catch {
        // Cancellation is best effort; the primary stream outcome wins.
      }
    }
    reader.releaseLock();
  }
}
