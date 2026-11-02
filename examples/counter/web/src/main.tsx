import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { ccc } from "@ckb-ccc/connector-react";
import App from "./App.tsx";
import { TESTNET_RPC } from "./lib/counter.ts";
import "./styles.css";

function Root() {
  const [client, setClient] = useState<ccc.Client>();
  useEffect(() => {
    const owner = ccc.ClientPublicTestnet.open({
      urls: [import.meta.env.VITE_CKB_RPC_URL || TESTNET_RPC],
      timeout: 15_000,
    });
    setClient(owner.value);
    return () => {
      void owner.dispose();
    };
  }, []);
  if (!client) return <div className="boot">Opening CKB Testnet…</div>;
  return (
    <ccc.Provider
      name="CKB Counter"
      defaultClient={client}
      clientOptions={[{ name: "CKB Testnet", client }]}
    >
      <App />
    </ccc.Provider>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
