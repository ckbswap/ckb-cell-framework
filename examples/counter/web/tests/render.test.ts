import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { ccc } from "@ckb-ccc/connector-react";
import App from "../src/App.tsx";

test("the disconnected Counter page renders without a wallet or deployment", async () => {
  const owner = ccc.ClientPublicTestnet.open({
    urls: ["http://127.0.0.1:1"],
    timeout: 100,
  });
  try {
    const html = renderToString(
      createElement(ccc.Provider, {
        defaultClient: owner.value,
        children: createElement(App),
      }),
    );
    assert.match(html, /A small number/);
    assert.match(html, /Connect wallet/);
    assert.match(html, /Your counter/);
    assert.match(html, /Create counter/);
    assert.match(html, /300 test CKB/);
    assert.match(html, /does not support closing/);
    assert.match(html, /CKB Testnet/);
  } finally {
    await owner.dispose();
  }
});
