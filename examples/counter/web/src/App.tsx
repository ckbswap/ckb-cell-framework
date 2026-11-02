import { useCallback, useEffect, useRef, useState } from "react";
import { ccc } from "@ckb-ccc/connector-react";
import {
  EXPLORER,
  displayError,
  findCounters,
  parseDeployment,
  parseIncrement,
  prepareCreate,
  prepareUpdate,
  sendPrepared,
  verifyDeployment,
  waitCommitted,
  type Counter,
  type Deployment,
} from "./lib/counter.ts";

const DEPLOYMENT_KEY = "ckb-counter:testnet:deployment:v2";
type Receipt = {
  hash: ccc.Hex;
  id: ccc.Hex;
  kind: "create" | "increment";
  status: "pending" | "committed" | "rejected";
  owner: ccc.Hex;
  codeHash: ccc.Hex;
};
function shorten(value: string, size = 8) {
  return `${value.slice(0, size + 2)}…${value.slice(-size)}`;
}
function remember(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* Storage is optional. */
  }
}
function recalled(key: string): unknown {
  try {
    return JSON.parse(localStorage.getItem(key) || "null");
  } catch {
    return null;
  }
}
function receiptKey(codeHash: string, owner: string) {
  return `ckb-counter:receipt:${codeHash}:${owner}`;
}
function Arrow({ diagonal = false }: { diagonal?: boolean }) {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d={diagonal ? "M6 18 18 6M6 6h12v12" : "M4 12h16m-6-6 6 6-6 6"}
        stroke="currentColor"
        strokeWidth="1.7"
      />
    </svg>
  );
}

export default function App() {
  const { open, wallet, client } = ccc.useCcc();
  const signer = ccc.useSigner();
  const [deployment, setDeployment] = useState<Deployment | null>(null);
  const [configReady, setConfigReady] = useState(false);
  const [counters, setCounters] = useState<Counter[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [address, setAddress] = useState("");
  const [ownerHash, setOwnerHash] = useState<ccc.Hex>();
  const [balance, setBalance] = useState<bigint>();
  const [increment, setIncrement] = useState("1");
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState("");
  const [error, setError] = useState("");
  const [receipt, setReceipt] = useState<Receipt | null>(null);
  const loadVersion = useRef(0);
  const session = useRef({ signer, deployment });
  session.current = { signer, deployment };
  const selected =
    counters.find((counter) => counter.id === selectedId) ?? counters[0];
  const currentReceipt =
    receipt?.codeHash === deployment?.codeHash && receipt?.owner === ownerHash
      ? receipt
      : null;
  const blocked = busy || loading || currentReceipt?.status === "pending";

  useEffect(() => {
    let active = true;
    (async () => {
      try {
        const stored = recalled(DEPLOYMENT_KEY);
        const response = stored
          ? null
          : await fetch(`${import.meta.env.BASE_URL}deployment.json`, {
              cache: "no-store",
            });
        if (response && !response.ok)
          throw new Error("Could not load deployment.json.");
        const value: unknown = stored || (await response!.json());
        if (active && value) setDeployment(parseDeployment(value));
      } catch (e) {
        if (active) setError(displayError(e));
      } finally {
        if (active) setConfigReady(true);
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    setAddress("");
    setOwnerHash(undefined);
    setBalance(undefined);
    setReceipt(null);
    if (signer) {
      void signer
        .getRecommendedAddressObj()
        .then((value) => {
          if (active) {
            setAddress(value.toString());
            setOwnerHash(value.script.hash());
          }
        })
        .catch((e) => {
          if (active) setError(displayError(e));
        });
      void signer
        .getBalance()
        .then((value) => {
          if (active) setBalance(value);
        })
        .catch(() => {});
    }
    return () => {
      active = false;
    };
  }, [signer]);

  const refresh = useCallback(
    async (preferred?: string) => {
      if (
        session.current.signer !== signer ||
        session.current.deployment !== deployment
      )
        return;
      const version = ++loadVersion.current;
      if (!signer || !client || !deployment) {
        setCounters([]);
        setLoading(false);
        return;
      }
      setLoading(true);
      try {
        await verifyDeployment(client, deployment);
        const owner = (await signer.getRecommendedAddressObj()).script.hash();
        const results = await findCounters(client, deployment, owner);
        if (version !== loadVersion.current) return;
        setCounters(results);
        setSelectedId(
          (current) =>
            results.find((item) => item.id === (preferred || current))?.id ||
            results[0]?.id ||
            "",
        );
        const stored = recalled(
          receiptKey(deployment.codeHash, owner),
        ) as Receipt | null;
        if (
          stored &&
          stored.codeHash === deployment.codeHash &&
          stored.owner === owner &&
          /^0x[0-9a-f]{64}$/.test(stored.hash)
        ) {
          const status = await client.getTransaction(stored.hash);
          if (status?.status === "committed" || status?.status === "rejected")
            stored.status = status.status;
          if (version === loadVersion.current) {
            setReceipt(stored);
            remember(receiptKey(deployment.codeHash, owner), stored);
          }
        }
        void signer
          .getBalance()
          .then((value) => {
            if (version === loadVersion.current) setBalance(value);
          })
          .catch(() => {});
      } finally {
        if (version === loadVersion.current) setLoading(false);
      }
    },
    [client, deployment, signer],
  );

  useEffect(() => {
    setCounters([]);
    void refresh().catch((e) => setError(displayError(e)));
    return () => {
      ++loadVersion.current;
    };
  }, [refresh]);

  async function importConfig(file?: File) {
    if (!file || !client) return;
    setError("");
    setBusy(true);
    setPhase("Verifying deployed code…");
    try {
      const value = parseDeployment(JSON.parse(await file.text()));
      await verifyDeployment(client, value);
      remember(DEPLOYMENT_KEY, value);
      setDeployment(value);
      setReceipt(null);
    } catch (e) {
      setError(displayError(e));
    } finally {
      setBusy(false);
      setPhase("");
    }
  }

  async function transact(kind: "create" | "increment") {
    if (!signer || !deployment || blocked) return;
    setBusy(true);
    setError("");
    setPhase("Preparing your transaction…");
    const isCurrent = () =>
      session.current.signer === signer &&
      session.current.deployment === deployment;
    try {
      const prepared =
        kind === "create"
          ? await prepareCreate(signer, deployment)
          : await prepareUpdate(
              signer,
              deployment,
              selected!,
              parseIncrement(increment),
            );
      if (!isCurrent()) return;
      setPhase(
        `Confirm in your wallet · fee ${ccc.fixedPointToString(prepared.fee)} CKB`,
      );
      const record: Receipt = {
        hash: prepared.tx.hash(),
        id: prepared.id,
        kind,
        status: "pending",
        owner: prepared.state.owner,
        codeHash: deployment.codeHash,
      };
      const hash = await sendPrepared(signer, prepared, () => {
        remember(receiptKey(record.codeHash, record.owner), record);
        if (isCurrent()) setReceipt(record);
      });
      if (isCurrent())
        setPhase("Transaction sent. Waiting for testnet confirmation…");
      await waitCommitted(signer.client, hash);
      record.status = "committed";
      remember(receiptKey(record.codeHash, record.owner), record);
      if (!isCurrent()) return;
      setReceipt({ ...record });
      await refresh(prepared.id);
      setPhase("Confirmed on CKB Testnet.");
    } catch (e) {
      if (isCurrent()) {
        setError(displayError(e));
        setPhase("");
      }
    } finally {
      setBusy(false);
    }
  }

  async function checkReceipt() {
    if (!client || !currentReceipt || busy) return;
    setBusy(true);
    setError("");
    setPhase("Checking transaction confirmation…");
    try {
      const tx = await client.getTransaction(currentReceipt.hash);
      if (tx?.status === "rejected") {
        const record = { ...currentReceipt, status: "rejected" as const };
        remember(receiptKey(record.codeHash, record.owner), record);
        setReceipt(record);
        throw new Error(
          tx.reason ||
            "Transaction rejected. Refresh your Counter before trying again.",
        );
      }
      await waitCommitted(client, currentReceipt.hash);
      const record = { ...currentReceipt, status: "committed" as const };
      remember(receiptKey(record.codeHash, record.owner), record);
      setReceipt(record);
      await refresh(record.id);
      setPhase("Confirmed on CKB Testnet.");
    } catch (e) {
      setError(displayError(e));
      setPhase("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <a className="brand" href="./" aria-label="Counter home">
          <span className="brand-mark">
            c<span>+</span>
          </span>
          <span>
            cell framework<span className="brand-divider">/</span>
            <strong>counter</strong>
          </span>
        </a>
        <div className="topbar-actions">
          <span className="network">
            <i /> CKB Testnet
          </span>
          <button
            className="wallet-button"
            onClick={open}
            title={signer ? "Manage connected wallet" : "Connect wallet"}
            disabled={busy}
          >
            <span className="wallet-icon" aria-hidden="true">
              ◈
            </span>
            {signer
              ? address
                ? shorten(address, 5)
                : wallet?.name || "Connected"
              : "Connect wallet"}
          </button>
        </div>
      </header>

      <main>
        <section className="intro">
          <div>
            <p className="eyebrow">
              <span className="tiny-square" /> THE COUNTER EXAMPLE
            </p>
            <h1>
              A small number.
              <br />
              <span>A real state change.</span>
            </h1>
            <p className="intro-copy">
              Your counter lives in a CKB cell. Every increment is
              <br className="desktop-break" /> signed by you and verified on the
              testnet.
            </p>
          </div>
          <div className="intro-index" aria-hidden="true">
            <span>EXAMPLE</span>
            <strong>
              01<span>/05</span>
            </strong>
            <div className="index-line" />
          </div>
        </section>

        {error && (
          <div className="notice error" role="alert">
            <span>{error}</span>
            <button aria-label="Dismiss error" onClick={() => setError("")}>
              ×
            </button>
          </div>
        )}

        {configReady && !deployment && (
          <section className="setup-callout">
            <div className="setup-symbol" aria-hidden="true">
              ↗
            </div>
            <div>
              <h2>Connect your deployed Counter</h2>
              <p>
                Import the deployment JSON to use your contract on CKB Testnet.
              </p>
            </div>
            <label
              className={`button secondary file-button ${busy ? "disabled" : ""}`}
            >
              Import deployment <Arrow diagonal />
              <input
                aria-label="Import deployment JSON"
                type="file"
                accept=".json,application/json"
                disabled={busy}
                onChange={(event) => {
                  void importConfig(event.target.files?.[0]);
                  event.target.value = "";
                }}
              />
            </label>
          </section>
        )}

        <div className="workspace">
          <section className="counter-panel" aria-labelledby="counter-heading">
            <div className="panel-heading">
              <h2 id="counter-heading">Your counter</h2>
              <span className="small-tag">STATE CELL</span>
            </div>
            <div className="counter-picker-row">
              {counters.length > 1 ? (
                <select
                  aria-label="Select counter"
                  value={selected?.id || ""}
                  disabled={blocked}
                  onChange={(event) => setSelectedId(event.target.value)}
                >
                  {counters.map((counter) => (
                    <option value={counter.id} key={counter.id}>
                      {shorten(counter.id, 6)}
                    </option>
                  ))}
                </select>
              ) : (
                <span className="counter-id">
                  {selected
                    ? shorten(selected.id, 9)
                    : signer
                      ? "Create a counter to begin"
                      : "Connect a wallet to begin"}
                </span>
              )}
              <button
                className={`icon-button ${loading ? "spinning" : ""}`}
                title="Refresh from testnet"
                aria-label="Refresh from testnet"
                disabled={!signer || !deployment || busy || loading}
                onClick={() => {
                  setError("");
                  void refresh().catch((e) => setError(displayError(e)));
                }}
              >
                ↻
              </button>
            </div>
            <div className="value-stage">
              <div
                className={`counter-value ${(selected?.state.value.toString().length || 0) > 10 ? "long-value" : ""}`}
                aria-live="polite"
              >
                {selected ? selected.state.value.toLocaleString("en-US") : "—"}
              </div>
              <span className="value-label">CURRENT VALUE</span>
              <div className="value-rule" />
            </div>
            <div className="counter-facts">
              <div>
                <span>Updates</span>
                <strong>{selected?.state.nonce.toString() ?? "—"}</strong>
              </div>
              <div>
                <span>Cell capacity</span>
                <strong>
                  {selected ? "300" : "—"}
                  <small> CKB</small>
                </strong>
              </div>
              <div>
                <span>Network</span>
                <strong>
                  <i className="status-dot" />
                  Testnet
                </strong>
              </div>
            </div>
            <div className="increment-area">
              <label htmlFor="increment">Increment by</label>
              <div className="increment-controls">
                <input
                  id="increment"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  autoComplete="off"
                  value={increment}
                  onChange={(event) => setIncrement(event.target.value)}
                  disabled={!selected || blocked}
                />
                <button
                  className="button primary"
                  disabled={!signer || !deployment || !selected || blocked}
                  onClick={() => void transact("increment")}
                >
                  Increment <span aria-hidden="true">+</span>
                </button>
              </div>
              <p>
                Positive whole numbers. Each update requires a wallet signature.
              </p>
            </div>
          </section>

          <aside className="sidebar">
            <section className="wallet-card">
              <div className="panel-heading">
                <h2>Wallet</h2>
                <span
                  className={`connection-dot ${signer ? "connected" : ""}`}
                />
              </div>
              {signer ? (
                <>
                  <p className="wallet-name">
                    {wallet?.name || "Connected wallet"}
                  </p>
                  <p className="address" title={address}>
                    {address ? shorten(address, 10) : "Loading address…"}
                  </p>
                  <div className="balance">
                    <strong>
                      {balance === undefined
                        ? "—"
                        : `${(balance / 100_000_000n).toLocaleString("en-US")}.${((balance % 100_000_000n) / 1_000_000n).toString().padStart(2, "0")}`}
                    </strong>
                    <span>CKB</span>
                  </div>
                  <a
                    className="text-link"
                    href={`${EXPLORER}/address/${address}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    View wallet <Arrow diagonal />
                  </a>
                </>
              ) : (
                <>
                  <div className="empty-wallet" aria-hidden="true">
                    ◈
                  </div>
                  <h3>Bring your own signature.</h3>
                  <p>
                    Connect a wallet to create a counter and update its state.
                  </p>
                  <button
                    className="button secondary full-width"
                    onClick={open}
                  >
                    Connect wallet <Arrow />
                  </button>
                </>
              )}
              <a
                className="faucet-link"
                href="https://faucet.nervos.org/"
                target="_blank"
                rel="noreferrer"
              >
                Need test CKB? Visit the faucet <span>↗</span>
              </a>
            </section>
            <section className="create-card">
              <span className="eyebrow">START A NEW STATE</span>
              <h2>Make it yours.</h2>
              <p>
                Create an independent counter owned by your connected wallet.
              </p>
              <button
                className="button dark full-width"
                disabled={!signer || !deployment || blocked}
                onClick={() => void transact("create")}
              >
                Create counter <Arrow />
              </button>
              <p className="capacity-note">
                Locks 300 test CKB, plus a small transaction fee. This example
                does not support closing the cell.
              </p>
            </section>
          </aside>
        </div>

        <section className="activity" aria-labelledby="activity-heading">
          <div className="panel-heading">
            <h2 id="activity-heading">Transaction activity</h2>
            <span className="small-tag">ON-CHAIN</span>
          </div>
          <div className="activity-content" aria-live="polite">
            {phase && (
              <p className="phase">
                {busy && <span className="loader" />}
                {phase}
              </p>
            )}
            {currentReceipt ? (
              <div className="receipt">
                <div>
                  <span className={`receipt-status ${currentReceipt.status}`}>
                    {currentReceipt.status}
                  </span>
                  <strong>
                    {currentReceipt.kind === "create"
                      ? "Counter created"
                      : "Counter increment"}
                  </strong>
                  <a
                    href={`${EXPLORER}/transaction/${currentReceipt.hash}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {shorten(currentReceipt.hash, 10)} <Arrow diagonal />
                  </a>
                </div>
                {currentReceipt.status === "pending" && (
                  <button
                    className="button secondary"
                    disabled={busy}
                    onClick={() => void checkReceipt()}
                  >
                    Check confirmation
                  </button>
                )}
              </div>
            ) : (
              !phase && (
                <p className="empty-activity">
                  Your next state change will appear here, with a transaction
                  you can verify.
                </p>
              )
            )}
          </div>
        </section>

        <details className="contract-details">
          <summary>
            <span>Contract details</span>
            <span>
              Deployment &amp; cell identity <span aria-hidden="true">+</span>
            </span>
          </summary>
          <div className="details-content">
            <div>
              <span>Code cell</span>
              {deployment ? (
                <a
                  href={`${EXPLORER}/transaction/${deployment.outPoint.txHash}`}
                  target="_blank"
                  rel="noreferrer"
                >
                  {shorten(deployment.outPoint.txHash, 12)}:
                  {deployment.outPoint.index} ↗
                </a>
              ) : (
                <p>No deployment configured.</p>
              )}
            </div>
            <div>
              <span>Code hash · data2</span>
              <code>{deployment?.codeHash || "—"}</code>
            </div>
            {selected && (
              <div>
                <span>Current state cell</span>
                <a
                  href={`${EXPLORER}/transaction/${selected.cell.outPoint.txHash}`}
                  target="_blank"
                  rel="noreferrer"
                >
                  {shorten(selected.cell.outPoint.txHash, 12)}:0 ↗
                </a>
              </div>
            )}
            <label
              className={`button secondary file-button ${busy ? "disabled" : ""}`}
            >
              Import deployment JSON{" "}
              <input
                aria-label="Change deployment JSON"
                type="file"
                accept=".json,application/json"
                disabled={busy}
                onChange={(event) => {
                  void importConfig(event.target.files?.[0]);
                  event.target.value = "";
                }}
              />
            </label>
          </div>
        </details>
      </main>
      <footer>
        <span>
          <span className="footer-mark">c+</span> CKB Cell Framework
        </span>
        <span>
          React + CCC <span className="footer-dot">·</span> Pudge testnet
        </span>
        <a
          href="https://github.com/ckb-devrel/ccc"
          target="_blank"
          rel="noreferrer"
        >
          Built with CCC ↗
        </a>
      </footer>
    </div>
  );
}
