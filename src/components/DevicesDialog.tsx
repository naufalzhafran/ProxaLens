// Set up phones, tablets and Simulators to use the proxy.
import { useEffect, useState } from "react";
import QRCode from "qrcode";
import { api, errorMessage, type DeviceInfo } from "../api";
import { copy, useToast } from "../lib/clipboard";
import { useSettings, useUi } from "../state/ui";
import { button, Modal, primaryButton } from "./ui";

export function DevicesDialog() {
  const close = useUi((s) => s.close);
  const [info, setInfo] = useState<DeviceInfo | null>(null);
  const [qr, setQr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const url = info?.setupUrls[0]?.[1] ?? null;

  const load = () => api.deviceInfo().then(setInfo).catch((e) => setError(errorMessage(e)));
  useEffect(() => {
    load();
  }, []);
  useEffect(() => {
    if (!url) return setQr(null);
    QRCode.toDataURL(url, { margin: 1, width: 180 }).then(setQr).catch(() => setQr(null));
  }, [url]);

  async function setAllowLan(allowLan: boolean) {
    setBusy(true);
    try {
      const s = useSettings.getState().settings ?? (await api.getSettings());
      await useSettings.getState().save({ ...s, allowLan });
      await load();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function installSimulator() {
    try {
      useToast.getState().show(await api.installSimulatorCert());
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <Modal title="Devices" onClose={close} footer={error && <span className="mr-auto text-red-500">{error}</span>}>
      <div className="flex flex-col gap-4">
        <section>
          <h3 className="mb-1 font-medium">iPhone, iPad and Android on the same network</h3>
          <label className="flex items-center gap-2">
            <input type="checkbox" disabled={busy || !info} checked={info?.allowLan ?? false} onChange={(e) => setAllowLan(e.target.checked)} />
            Allow devices on the network to use the proxy
          </label>
          <p className="mt-1 text-amber-700 dark:text-amber-300">
            Anyone who can reach this Mac can then use the proxy. Switch it off when you’re done, especially on public networks.
          </p>
          {info?.allowLan && (
            <div className="mt-3 flex gap-4">
              {qr && <img src={qr} alt="QR code for the setup page" className="h-[180px] w-[180px] rounded bg-white" />}
              <div className="flex min-w-0 flex-col gap-2">
                <p>On the device, open the setup page (scan the code) to install the certificate, then set its Wi-Fi proxy to:</p>
                {info.setupUrls.length === 0 && <p className="text-neutral-500">No network connection found.</p>}
                {info.setupUrls.map(([iface, u]) => {
                  const hostPort = u.replace(/^http:\/\//, "").replace(/\/$/, "");
                  return (
                    <div key={iface} className="flex items-center gap-2 font-mono">
                      <span className="text-neutral-500">{iface}</span>
                      <span className="selectable">{hostPort}</span>
                      <button className={button} onClick={() => copy(u, "setup URL")}>
                        Copy URL
                      </button>
                    </div>
                  );
                })}
                <p className="text-neutral-500">
                  iOS: after installing the profile, enable it under Settings → General → About → Certificate Trust Settings.
                </p>
              </div>
            </div>
          )}
        </section>
        <section>
          <h3 className="mb-1 font-medium">iOS Simulator</h3>
          <p className="mb-2 text-neutral-600 dark:text-neutral-400">
            Simulators use this Mac’s proxy settings, so switch the proxy on and trust the certificate in each booted Simulator.
          </p>
          <button className={primaryButton} onClick={installSimulator}>
            Install certificate in booted Simulators
          </button>
        </section>
      </div>
    </Modal>
  );
}
