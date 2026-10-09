import { PassThrough } from "node:stream";
import type { ReactNode } from "react";
import { renderToPipeableStream } from "react-dom/server";

export async function renderAsyncPage(
  element: ReactNode | PromiseLike<ReactNode>,
): Promise<string> {
  const resolvedElement = await element;
  return new Promise((resolve, reject) => {
    const output = new PassThrough();
    const chunks: Buffer[] = [];
    let renderer: ReturnType<typeof renderToPipeableStream> | null = null;
    let settled = false;
    let readyToPipe = false;
    let piped = false;

    const pipeWhenReady = () => {
      if (settled || !readyToPipe || piped || renderer === null) return;
      piped = true;
      renderer.pipe(output);
    };

    output.on("data", (chunk: Buffer | string) => {
      chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
    });
    output.on("error", (error: Error) => {
      if (settled) return;
      settled = true;
      renderer?.abort();
      reject(error);
    });
    output.on("end", () => {
      if (settled) return;
      settled = true;
      resolve(Buffer.concat(chunks).toString("utf8"));
    });

    renderer = renderToPipeableStream(resolvedElement, {
      onAllReady() {
        readyToPipe = true;
        pipeWhenReady();
      },
      onError() {
        // A Suspense boundary can recover with its fallback; the completed stream is the oracle.
      },
      onShellError(error) {
        if (settled) return;
        settled = true;
        renderer?.abort();
        output.destroy();
        reject(error);
      },
    });
    pipeWhenReady();
  });
}
