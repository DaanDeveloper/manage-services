import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Plus,
  Power,
  RefreshCw,
  Settings,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { CommandResult } from "./types";

type ThemeChoice = "system" | "light" | "dark";

function getSystemTheme(): "light" | "dark" {
  if (typeof window === "undefined" || !window.matchMedia) return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

type RedisInstance = {
  id: string;
  kind: "redis" | "typesense" | "mysql";
  name: string;
  host: string;
  port: number;
  version: string;
  apiKey?: string;
  socket?: string;
  mysqlConfig?: string;
  managed?: boolean;
  status: "running" | "stopped" | "checking";
};

type Preferences = {
  launchOnLogin: boolean;
  theme: ThemeChoice;
};

type DiscoveredService = {
  kind: "redis" | "typesense" | "mysql";
  name: string;
  host: string;
  port: number;
  version: string;
  apiKey?: string;
  socket?: string;
};

type PortPayload = {
  port: number;
};

type ProcessInfo = {
  pid: number;
  name: string;
  detail: string;
  port?: number;
};

const defaultInstances: RedisInstance[] = [
  {
    id: "redis-6379",
    kind: "redis",
    name: "redis",
    host: "127.0.0.1",
    port: 6379,
    version: "Redis",
    status: "checking",
  },
  {
    id: "typesense-8108",
    kind: "typesense",
    name: "typesense",
    host: "127.0.0.1",
    port: 8108,
    version: "Typesense",
    apiKey: "change-me",
    status: "checking",
  },
  {
    id: "mysql-3306",
    kind: "mysql",
    name: "mysql",
    host: "127.0.0.1",
    port: 3306,
    version: "MySQL",
    socket: "/tmp/mysql.sock",
    status: "checking",
  },
  {
    id: "mysql-3308-xampp",
    kind: "mysql",
    name: "xampp mysql",
    host: "127.0.0.1",
    port: 3308,
    version: "MySQL",
    socket: "/Applications/XAMPP/xamppfiles/var/mysql/mysql.sock",
    status: "checking",
  },
];

function loadJson<T>(key: string, fallback: T): T {
  try {
    const value = localStorage.getItem(key);
    return value ? (JSON.parse(value) as T) : fallback;
  } catch {
    return fallback;
  }
}

function normalizeInstances(instances: RedisInstance[]): RedisInstance[] {
  return instances.map((instance) => {
    const kind =
      instance.kind ??
      (instance.version.toLowerCase().includes("typesense")
        ? "typesense"
        : instance.version.toLowerCase().includes("mysql")
          ? "mysql"
          : "redis");

    const socket =
      kind === "mysql"
        ? instance.socket ??
          (instance.port === 3308
            ? "/Applications/XAMPP/xamppfiles/var/mysql/mysql.sock"
            : instance.port === 3306
              ? "/tmp/mysql.sock"
              : undefined)
        : undefined;

    return {
      ...instance,
      kind,
      apiKey: kind === "typesense" ? instance.apiKey ?? "change-me" : undefined,
      mysqlConfig: kind === "mysql" ? instance.mysqlConfig : undefined,
      managed: Boolean(instance.managed),
      socket,
    };
  });
}

function serviceKey(kind: RedisInstance["kind"], port: number) {
  return `${kind}:${port}`;
}

function cleanMysqlConfigValue(value: string) {
  return value
    .replace(/\s[#;].*$/, "")
    .trim()
    .replace(/^["']|["']$/g, "");
}

function defaultMysqlConfig(port: number, socket?: string) {
  return [
    "[mysqld]",
    `port=${port}`,
    socket ? `socket=${socket}` : "",
    "skip-networking=0",
    "",
  ]
    .filter((line) => line !== "")
    .join("\n");
}

function readMysqlConfigValue(config: string, key: string) {
  let inMysqld = false;
  const normalizedKey = key.toLowerCase();

  for (const line of config.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith(";")) continue;

    const section = trimmed.match(/^\[([^\]]+)\]$/);
    if (section) {
      inMysqld = section[1].toLowerCase() === "mysqld";
      continue;
    }

    if (!inMysqld) continue;

    const [rawKey, ...valueParts] = trimmed.split("=");
    if (rawKey.trim().toLowerCase().replace(/-/g, "_") !== normalizedKey) continue;

    return cleanMysqlConfigValue(valueParts.join("="));
  }

  return undefined;
}

function upsertMysqlConfigValue(config: string, key: string, value: string) {
  const lines = config.split(/\r?\n/);
  let inMysqld = false;
  let mysqldIndex = -1;
  const normalizedKey = key.toLowerCase();

  for (let index = 0; index < lines.length; index += 1) {
    const trimmed = lines[index].trim();
    const section = trimmed.match(/^\[([^\]]+)\]$/);

    if (section) {
      inMysqld = section[1].toLowerCase() === "mysqld";
      if (inMysqld) mysqldIndex = index;
      continue;
    }

    if (!inMysqld || trimmed.startsWith("#") || trimmed.startsWith(";")) continue;

    const [rawKey] = trimmed.split("=");
    if (rawKey.trim().toLowerCase().replace(/-/g, "_") === normalizedKey) {
      lines[index] = `${key}=${value}`;
      return lines.join("\n");
    }
  }

  if (mysqldIndex >= 0) {
    lines.splice(mysqldIndex + 1, 0, `${key}=${value}`);
    return lines.join("\n");
  }

  return [`[mysqld]`, `${key}=${value}`, config].filter(Boolean).join("\n");
}

function App() {
  const [instances, setInstances] = useState<RedisInstance[]>(() =>
    normalizeInstances(loadJson("service-desk.redisInstances", defaultInstances)),
  );
  const [preferences, setPreferences] = useState<Preferences>(() =>
    loadJson("service-desk.preferences", {
      launchOnLogin: false,
      theme: "system" as ThemeChoice,
    }),
  );
  const [showSettings, setShowSettings] = useState(false);
  const settingsRef = useRef<HTMLDivElement>(null);
  const settingsButtonRef = useRef<HTMLButtonElement>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [newKind, setNewKind] = useState<"redis" | "typesense" | "mysql">("redis");
  const [addTypeOpen, setAddTypeOpen] = useState(false);
  const addTypeRef = useRef<HTMLDivElement>(null);
  const addRowRef = useRef<HTMLElement>(null);
  const addButtonRef = useRef<HTMLButtonElement>(null);
  const [newName, setNewName] = useState("redis");
  const [newPort, setNewPort] = useState(6380);
  const [newApiKey, setNewApiKey] = useState("change-me");
  const [newSocket, setNewSocket] = useState("");
  const [message, setMessage] = useState("Ready");
  const [activeTab, setActiveTab] = useState<"services" | "processes">("services");
  const [servicesOpen, setServicesOpen] = useState(false);
  const [otherOpen, setOtherOpen] = useState(false);
  const [processesOpen, setProcessesOpen] = useState(false);
  const [processes, setProcesses] = useState<ProcessInfo[]>([]);
  const [hiddenServices, setHiddenServices] = useState<string[]>(() =>
    loadJson("service-desk.hiddenServices", []),
  );
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState("");
  const [configuring, setConfiguring] = useState<RedisInstance | null>(null);
  const [configName, setConfigName] = useState("");
  const [configPort, setConfigPort] = useState("");
  const [configSocket, setConfigSocket] = useState("");
  const [configText, setConfigText] = useState("");
  const [confirmRemove, setConfirmRemove] = useState<RedisInstance | null>(null);

  useEffect(() => {
    localStorage.setItem("service-desk.redisInstances", JSON.stringify(instances));
  }, [instances]);

  useEffect(() => {
    localStorage.setItem("service-desk.hiddenServices", JSON.stringify(hiddenServices));
  }, [hiddenServices]);

  useEffect(() => {
    localStorage.setItem("service-desk.preferences", JSON.stringify(preferences));
  }, [preferences]);

  useEffect(() => {
    const root = document.documentElement;
    const apply = () => {
      const resolved =
        preferences.theme === "system" ? getSystemTheme() : preferences.theme;
      root.dataset.theme = resolved;
      root.style.colorScheme = resolved;
    };
    apply();
    if (preferences.theme !== "system") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [preferences.theme]);

  useEffect(() => {
    if (!addTypeOpen) return;
    const handler = (event: MouseEvent) => {
      if (addTypeRef.current?.contains(event.target as Node)) return;
      setAddTypeOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [addTypeOpen]);

  useEffect(() => {
    if (!showAdd) return;
    const handler = (event: MouseEvent) => {
      if (addTypeOpen) return;
      const target = event.target as Node;
      if (addRowRef.current?.contains(target)) return;
      if (addButtonRef.current?.contains(target)) return;
      setShowAdd(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showAdd, addTypeOpen]);

  useEffect(() => {
    if (!showSettings) return;
    const handler = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        settingsRef.current?.contains(target) ||
        settingsButtonRef.current?.contains(target)
      ) {
        return;
      }
      setShowSettings(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showSettings]);

  useEffect(() => {
    void refreshAll();
    void unlockXamppAdmin();
  }, []);

  async function quitApp() {
    try {
      await invoke("quit_app");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  function updateInstance(id: string, patch: Partial<RedisInstance>) {
    setInstances((current) =>
      current.map((instance) => (instance.id === id ? { ...instance, ...patch } : instance)),
    );
  }

  async function refreshInstance(instance: RedisInstance) {
    updateInstance(instance.id, { status: "checking" });

    try {
      const result =
        instance.kind === "redis"
          ? await invoke<CommandResult>("test_redis_connection", {
              host: instance.host,
              port: instance.port,
            })
          : instance.kind === "typesense"
            ? await invoke<CommandResult>("test_typesense_connection", {
              host: instance.host,
              port: instance.port,
              apiKey: instance.apiKey ?? "change-me",
              protocol: "http",
            })
            : await invoke<CommandResult>("test_mysql_connection", {
                host: instance.host,
                port: instance.port,
                socket: instance.socket,
              });

      updateInstance(instance.id, { status: result.ok ? "running" : "stopped" });
    } catch {
      updateInstance(instance.id, { status: "stopped" });
    }
  }

  async function syncDiscoveredServices(currentInstances = instances) {
    try {
      const discovered = await invoke<DiscoveredService[]>("discover_running_services");
      const known = new Map(currentInstances.map((instance) => [`${instance.kind}:${instance.port}`, instance]));

      for (const service of discovered) {
        const key = `${service.kind}:${service.port}`;
        if (hiddenServices.includes(key)) {
          continue;
        }
        const existing = known.get(key);

        known.set(key, {
          id: existing?.id ?? `${service.kind}-${service.port}-discovered`,
          kind: service.kind,
          name: existing?.name ?? service.name,
          host: service.host,
          port: service.port,
          version: service.version,
          apiKey: existing?.apiKey ?? service.apiKey,
          managed: existing?.managed ?? false,
          socket: existing?.socket ?? service.socket,
          status: "running",
        });
      }

      const merged = Array.from(known.values()).sort((left, right) => left.port - right.port);
      setInstances(merged);
      return merged;
    } catch {
      return currentInstances;
    }
  }

  async function refreshAll() {
    setMessage("Checking services...");
    const merged = await syncDiscoveredServices();
    await refreshProcesses();
    await Promise.all(merged.map((instance) => refreshInstance(instance)));
    setMessage("Status updated");
  }

  async function refreshProcesses() {
    try {
      const result = await invoke<ProcessInfo[]>("discover_processes");
      setProcesses(result);
    } catch {
      setProcesses([]);
    }
  }

  async function inferMysqlPort(socket: string) {
    const cleanSocket = socket.trim();
    if (!cleanSocket) {
      return;
    }

    try {
      const result = await invoke<CommandResult<PortPayload>>("infer_mysql_port_from_socket", {
        socket: cleanSocket,
      });

      if (result.ok && result.data?.port) {
        setNewPort(result.data.port);
        setMessage(result.message);
      }
    } catch {
      // Socket port inference is a convenience; manual port input remains authoritative.
    }
  }

  async function unlockXamppAdmin() {
    try {
      await invoke<CommandResult>("unlock_xampp_admin");
    } catch {
      // XAMPP admin unlock is best-effort; direct Start/Stop still falls back to macOS auth.
    }
  }

  async function startService(instance: RedisInstance) {
    updateInstance(instance.id, { status: "checking" });
    setMessage(`Starting ${instance.name}...`);

    try {
      const result =
        instance.kind === "redis"
          ? await invoke<CommandResult>("start_redis_instance", {
              port: instance.port,
              name: instance.name,
            })
          : instance.kind === "typesense"
            ? await invoke<CommandResult>("start_typesense_instance", {
              port: instance.port,
              name: instance.name,
              apiKey: instance.apiKey ?? "change-me",
            })
            : await invoke<CommandResult>("start_mysql_instance", {
                port: instance.port,
                name: instance.name,
                config: instance.mysqlConfig,
              });

      setMessage(result.message);
      updateInstance(instance.id, { status: result.ok ? "running" : "stopped" });
      if (!result.ok) {
        await refreshInstance(instance);
      }
    } catch (error) {
      updateInstance(instance.id, { status: "stopped" });
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function stopService(instance: RedisInstance) {
    updateInstance(instance.id, { status: "checking" });
    setMessage(`Stopping ${instance.name}...`);

    try {
      const result =
        instance.kind === "redis"
          ? await invoke<CommandResult>("stop_redis_instance", {
              host: instance.host,
              port: instance.port,
            })
          : instance.kind === "typesense"
            ? await invoke<CommandResult>("stop_typesense_instance", {
              port: instance.port,
            })
            : await invoke<CommandResult>("stop_mysql_instance", {
                port: instance.port,
                name: instance.name,
              });

      setMessage(result.message);
      updateInstance(instance.id, { status: result.ok ? "stopped" : "running" });
      if (!result.ok) {
        await refreshInstance(instance);
      }
    } catch (error) {
      updateInstance(instance.id, { status: "running" });
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function addService() {
    const port = Number(newPort);
    if (!newName.trim() || !Number.isInteger(port) || port < 1 || port > 65535) {
      setMessage("Use a name and a valid port.");
      return;
    }

    if (instances.some((instance) => instance.port === port)) {
      setMessage(`Port ${port} is already in the list.`);
      return;
    }

    const managed = true;

    if (managed) {
      setMessage(
        newKind === "mysql"
          ? `Preparing ${newName.trim()}… (installs MySQL via Homebrew if missing — may take a few minutes)`
          : `Preparing ${newName.trim()}…`,
      );
      try {
        const result = await invoke<CommandResult>("create_managed_service", {
          kind: newKind,
          port,
          apiKey: newKind === "typesense" ? newApiKey : undefined,
        });

        if (!result.ok) {
          setMessage(result.message);
          return;
        }
      } catch (error) {
        setMessage(error instanceof Error ? error.message : String(error));
        return;
      }
    }

    const instance: RedisInstance = {
      id: `${newKind}-${port}-${Date.now()}`,
      kind: newKind,
      name: newName.trim(),
      host: "127.0.0.1",
      port,
      version: newKind === "redis" ? "Redis" : newKind === "typesense" ? "Typesense" : "MySQL",
      apiKey: newKind === "typesense" ? newApiKey : undefined,
      mysqlConfig: newKind === "mysql" ? defaultMysqlConfig(port) : undefined,
      managed,
      socket: undefined,
      status: "stopped",
    };

    setHiddenServices((current) => current.filter((entry) => entry !== serviceKey(instance.kind, instance.port)));
    setInstances((current) => [...current, instance]);
    setNewName(newKind);
    setNewPort(port + 1);
    setNewSocket("");
    setShowAdd(false);
    setMessage(`${instance.name} added`);
  }

  async function removeService(id: string) {
    const instance = instances.find((entry) => entry.id === id);
    if (!instance) {
      return;
    }

    if (instance.managed) {
      try {
        const result = await invoke<CommandResult>("delete_managed_service", {
          kind: instance.kind,
          port: instance.port,
        });

        if (!result.ok) {
          setMessage(result.message);
          return;
        }
      } catch (error) {
        setMessage(error instanceof Error ? error.message : String(error));
        return;
      }
    }

    setHiddenServices((hidden) => [...new Set([...hidden, serviceKey(instance.kind, instance.port)])]);
    setInstances((current) => current.filter((entry) => entry.id !== id));
    setMessage(instance.managed ? "Managed service deleted." : "Service removed from the list");
  }

  function beginRename(instance: RedisInstance) {
    setEditingId(instance.id);
    setEditingName(instance.name);
  }

  function saveRename() {
    if (!editingId) {
      return;
    }

    const name = editingName.trim();
    if (!name) {
      setMessage("Name cannot be empty.");
      return;
    }

    updateInstance(editingId, { name });
    setEditingId(null);
    setEditingName("");
    setMessage("Name updated");
  }

  function cancelRename() {
    setEditingId(null);
    setEditingName("");
  }

  async function beginConfigure(instance: RedisInstance) {
    const fallbackConfig = instance.mysqlConfig ?? defaultMysqlConfig(instance.port, instance.socket);
    setConfiguring(instance);
    setConfigName(instance.name);
    setConfigPort(String(instance.port));
    setConfigSocket(instance.socket ?? "");
    setConfigText(fallbackConfig);

    try {
      const result = await invoke<CommandResult<string>>("read_mysql_config", {
        port: instance.port,
        socket: instance.socket,
        managed: Boolean(instance.managed),
      });

      if (!result.ok || !result.data) {
        setMessage(result.message);
        return;
      }

      applyMysqlConfigContent(result.data);
      setMessage(result.message);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  function cancelConfigure() {
    setConfiguring(null);
    setConfigName("");
    setConfigPort("");
    setConfigSocket("");
    setConfigText("");
  }

  function updateConfigPort(value: string) {
    setConfigPort(value);
    if (/^\d+$/.test(value)) {
      setConfigText((current) => upsertMysqlConfigValue(current, "port", value));
    }
  }

  function updateConfigSocket(value: string) {
    setConfigSocket(value);
    setConfigText((current) =>
      value.trim() ? upsertMysqlConfigValue(current, "socket", value.trim()) : current,
    );
  }

  function applyMysqlConfigContent(content: string) {
    const loadedPort = readMysqlConfigValue(content, "port");
    const loadedSocket = readMysqlConfigValue(content, "socket");

    setConfigText(content);
    if (loadedPort) setConfigPort(loadedPort);
    if (loadedSocket) setConfigSocket(loadedSocket);
  }

  async function openMysqlConfigDocument() {
    if (!configuring) {
      return;
    }

    try {
      const result = await invoke<CommandResult<string>>("open_mysql_config", {
        port: configuring.port,
        socket: configuring.socket,
        managed: Boolean(configuring.managed),
        config: configText,
      });

      if (result.data) {
        applyMysqlConfigContent(result.data);
      }
      setMessage(result.message);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function saveMysqlConfig() {
    if (!configuring) {
      return;
    }

    const name = configName.trim();
    const configPortValue = readMysqlConfigValue(configText, "port");
    const configSocketValue = readMysqlConfigValue(configText, "socket");
    const port = Number(configPortValue ?? configPort);
    const socket = (configSocketValue ?? configSocket).trim();

    if (!name || !Number.isInteger(port) || port < 1 || port > 65535) {
      setMessage("Use a name and a valid MySQL port.");
      return;
    }

    if (instances.some((instance) => instance.id !== configuring.id && instance.port === port)) {
      setMessage(`Port ${port} is already in the list.`);
      return;
    }

    if (port !== configuring.port && configuring.status !== "stopped") {
      setMessage("Stop MySQL before changing its port.");
      return;
    }

    if (configuring.managed && port !== configuring.port) {
      try {
        const result = await invoke<CommandResult>("move_managed_mysql_instance", {
          oldPort: configuring.port,
          newPort: port,
        });

        if (!result.ok) {
          setMessage(result.message);
          return;
        }
      } catch (error) {
        setMessage(error instanceof Error ? error.message : String(error));
        return;
      }
    }

    const oldKey = serviceKey(configuring.kind, configuring.port);
    const newKey = serviceKey(configuring.kind, port);

    setHiddenServices((current) => current.filter((entry) => entry !== oldKey && entry !== newKey));
    updateInstance(configuring.id, {
      name,
      port,
      socket: socket || undefined,
      mysqlConfig: upsertMysqlConfigValue(configText.trim(), "port", String(port)),
      status: "stopped",
    });
    cancelConfigure();
    setMessage(port === configuring.port ? "MySQL config updated" : `MySQL config updated; app port is now ${port}.`);
  }

  async function stopProcess(process: ProcessInfo) {
    setMessage(`Stopping ${process.name} (${process.pid})...`);

    try {
      const result = await invoke<CommandResult>("stop_process", { pid: process.pid });
      setMessage(result.message);
      await refreshAll();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  function selectAddKind(kind: "redis" | "typesense" | "mysql") {
    setNewKind(kind);
    setNewName(kind);
    setNewPort(kind === "redis" ? 6380 : kind === "typesense" ? 8108 : 3306);
    setNewSocket("");
    setAddTypeOpen(false);
  }

  return (
    <main className="desk">
      <section className="deskBody">
        <div className="topControls">
          <div className="topControlsLeft">
            <button className="iconButton" type="button" title="Refresh" onClick={refreshAll}>
              <RefreshCw size={15} />
            </button>
            <button
              ref={addButtonRef}
              className={`iconButton${showAdd ? " active" : ""}`}
              type="button"
              title="Add service"
              onClick={() => setShowAdd((value) => !value)}
            >
              <Plus size={16} />
            </button>
            <span className="topDivider" aria-hidden />
            <label className="titleToggle">
              <input
                checked={preferences.launchOnLogin}
                type="checkbox"
                onChange={(event) =>
                  setPreferences((current) => ({ ...current, launchOnLogin: event.target.checked }))
                }
              />
              <span className="toggleBox" aria-hidden>
                <Check size={11} strokeWidth={3} />
              </span>
              <span className="toggleLabel">Launch on Login</span>
            </label>
          </div>
          <div className="topControlsRight">
            <div className="settingsWrapper">
              <button
                ref={settingsButtonRef}
                className={`iconButton${showSettings ? " active" : ""}`}
                type="button"
                title="Settings"
                onClick={() => setShowSettings((value) => !value)}
              >
                <Settings size={15} />
              </button>
              {showSettings && (
                <div ref={settingsRef} className="settingsPanel" role="dialog" aria-label="Settings">
                  <div className="settingsSection">
                    <div className="settingsLabel">Appearance</div>
                    <div className="segmented" role="radiogroup" aria-label="Theme">
                      {(["system", "light", "dark"] as const).map((option) => (
                        <button
                          key={option}
                          type="button"
                          role="radio"
                          aria-checked={preferences.theme === option}
                          className={preferences.theme === option ? "active" : ""}
                          onClick={() =>
                            setPreferences((current) => ({ ...current, theme: option }))
                          }
                        >
                          {option === "system" ? "Auto" : option === "light" ? "Light" : "Dark"}
                        </button>
                      ))}
                    </div>
                  </div>
                  <div className="settingsDivider" />
                  <button className="settingsQuit" type="button" onClick={quitApp}>
                    <Power size={13} />
                    <span>Quit Service Desk</span>
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>

        <nav className="tabBar" aria-label="Sections">
          <button className={activeTab === "services" ? "active" : ""} type="button" onClick={() => setActiveTab("services")}>Services</button>
          <button className={activeTab === "processes" ? "active" : ""} type="button" onClick={() => setActiveTab("processes")}>Processes</button>
        </nav>

        {showAdd && (
          <section ref={addRowRef} className="addRow">
            <div className="addField">
              <label className="addLabel">Type</label>
              <div ref={addTypeRef} className="customSelect">
                <button type="button" onClick={() => setAddTypeOpen((value) => !value)}>
                  <span>{newKind === "redis" ? "Redis" : newKind === "typesense" ? "Typesense" : "MySQL"}</span>
                  <ChevronDown size={14} />
                </button>
                {addTypeOpen && (
                  <div className="customSelectMenu">
                    <button type="button" onClick={() => selectAddKind("redis")}>Redis</button>
                    <button type="button" onClick={() => selectAddKind("typesense")}>Typesense</button>
                    <button type="button" onClick={() => selectAddKind("mysql")}>MySQL</button>
                  </div>
                )}
              </div>
            </div>
            <div className="addField">
              <label className="addLabel" htmlFor="add-name">Name</label>
              <input
                id="add-name"
                value={newName}
                onChange={(event) => setNewName(event.target.value)}
                placeholder="my-service"
              />
            </div>
            <div className="addField">
              <label className="addLabel" htmlFor="add-port">Port</label>
              <input
                id="add-port"
                value={newPort}
                min={1}
                max={65535}
                type="number"
                onChange={(event) => setNewPort(Number(event.target.value))}
                placeholder="6379"
              />
            </div>
            {newKind === "typesense" && (
              <div className="addField">
                <label className="addLabel" htmlFor="add-apikey">API key</label>
                <input
                  id="add-apikey"
                  value={newApiKey}
                  onChange={(event) => setNewApiKey(event.target.value)}
                  placeholder="change-me"
                />
              </div>
            )}
            <div className="addActions">
              <button className="smallAction" type="button" onClick={() => setShowAdd(false)}>
                Cancel
              </button>
              <button className="smallAction primary" type="button" onClick={addService}>
                <Check size={14} />
                Add service
              </button>
            </div>
          </section>
        )}

        {activeTab === "services" && <section className="serviceGroup">
          <button className="groupHeader" type="button" onClick={() => setServicesOpen((value) => !value)}>
            {servicesOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
            <span>Databases</span>
            <small>
              {instances.filter((instance) => instance.kind !== "typesense" && instance.status === "running").length}/
              {instances.filter((instance) => instance.kind !== "typesense").length} running
            </small>
          </button>

          {servicesOpen && <div className="serviceList">
            {instances.filter((instance) => instance.kind !== "typesense").map((instance) => (
              <article className="serviceItem" key={instance.id}>
                <div className={`serviceIcon ${instance.kind}`}>
                  {instance.kind === "redis" ? "Re" : "My"}
                </div>
                <div className="serviceMeta">
                  {editingId === instance.id ? (
                    <input
                      autoFocus
                      className="nameEdit"
                      value={editingName}
                      onBlur={saveRename}
                      onChange={(event) => setEditingName(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          saveRename();
                        }
                        if (event.key === "Escape") {
                          cancelRename();
                        }
                      }}
                    />
                  ) : (
                    <button className="nameButton" type="button" onClick={() => beginRename(instance)}>
                      {instance.name}
                    </button>
                  )}
                  <span>
                    {instance.version} : {instance.port}
                    {instance.socket ? ` · ${instance.socket}` : ""}
                  </span>
                </div>
                <div className={`dot ${instance.status}`} />
                <div className="serviceActions">
                  <button
                    className="serviceControl"
                    disabled={instance.status === "checking"}
                    type="button"
                    onClick={() =>
                      instance.status === "running" ? stopService(instance) : startService(instance)
                    }
                  >
                    {instance.status === "running" ? "Stop" : instance.status === "checking" ? "..." : "Start"}
                  </button>
                  {instance.kind === "mysql" && (
                    <button
                      className="configButton"
                      type="button"
                      title="Configure MySQL"
                      onClick={() => void beginConfigure(instance)}
                    >
                      <Settings size={14} />
                    </button>
                  )}
                  <button className="removeButton" type="button" title="Remove" onClick={() => setConfirmRemove(instance)}>
                    <X size={14} />
                  </button>
                </div>
              </article>
            ))}
          </div>}
        </section>}

        {activeTab === "processes" && <section className="serviceGroup">
          <button className="groupHeader" type="button" onClick={() => setProcessesOpen((value) => !value)}>
            {processesOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
            <span>Processes</span>
            <small>{processes.length} listening</small>
          </button>

          {processesOpen && <div className="serviceList">
            {processes.map((process) => (
              <article className="serviceItem" key={`${process.pid}-${process.port ?? process.detail}`}>
                <div className="serviceIcon process">Pr</div>
                <div className="serviceMeta">
                  <button className="nameButton" type="button">
                    {process.name}
                  </button>
                  <span>
                    PID {process.pid}
                    {process.port ? ` : ${process.port}` : ""} · {process.detail}
                  </span>
                </div>
                <div className="dot running" />
                <div className="serviceActions">
                  <button className="serviceControl" type="button" onClick={() => stopProcess(process)}>
                    Stop
                  </button>
                </div>
              </article>
            ))}
          </div>}
        </section>}

        {activeTab === "services" && <section className="serviceGroup">
          <button className="groupHeader" type="button" onClick={() => setOtherOpen((value) => !value)}>
            {otherOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
            <span>Other</span>
            <small>
              {instances.filter((instance) => instance.kind === "typesense" && instance.status === "running").length}/
              {instances.filter((instance) => instance.kind === "typesense").length} running
            </small>
          </button>

          {otherOpen && <div className="serviceList">
            {instances.filter((instance) => instance.kind === "typesense").map((instance) => (
              <article className="serviceItem" key={instance.id}>
                <div className="serviceIcon typesense">Ts</div>
                <div className="serviceMeta">
                  {editingId === instance.id ? (
                    <input
                      autoFocus
                      className="nameEdit"
                      value={editingName}
                      onBlur={saveRename}
                      onChange={(event) => setEditingName(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          saveRename();
                        }
                        if (event.key === "Escape") {
                          cancelRename();
                        }
                      }}
                    />
                  ) : (
                    <button className="nameButton" type="button" onClick={() => beginRename(instance)}>
                      {instance.name}
                    </button>
                  )}
                  <span>
                    {instance.version} : {instance.port}
                    {instance.socket ? ` · ${instance.socket}` : ""}
                  </span>
                </div>
                <div className={`dot ${instance.status}`} />
                <div className="serviceActions">
                  <button
                    className="serviceControl"
                    disabled={instance.status === "checking"}
                    type="button"
                    onClick={() =>
                      instance.status === "running" ? stopService(instance) : startService(instance)
                    }
                  >
                    {instance.status === "running" ? "Stop" : instance.status === "checking" ? "..." : "Start"}
                  </button>
                  <button className="removeButton" type="button" title="Remove" onClick={() => setConfirmRemove(instance)}>
                    <X size={14} />
                  </button>
                </div>
              </article>
            ))}
          </div>}
        </section>}

        <footer className="statusbar">{message}</footer>
      </section>

      {configuring && (
        <div className="modalOverlay" onMouseDown={cancelConfigure}>
          <div
            className="modal configModal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="config-title"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <h3 id="config-title" className="modalTitle">
              Configure {configuring.name}
            </h3>
            <div className="configFields">
              <label className="configField" htmlFor="mysql-config-name">
                <span>Name</span>
                <input
                  id="mysql-config-name"
                  value={configName}
                  onChange={(event) => setConfigName(event.target.value)}
                />
              </label>
              <label className="configField" htmlFor="mysql-config-port">
                <span>Port</span>
                <input
                  id="mysql-config-port"
                  value={configPort}
                  min={1}
                  max={65535}
                  type="number"
                  disabled={configuring.status !== "stopped"}
                  onChange={(event) => updateConfigPort(event.target.value)}
                />
              </label>
              <label className="configField" htmlFor="mysql-config-socket">
                <span>Socket</span>
                <input
                  id="mysql-config-socket"
                  value={configSocket}
                  onChange={(event) => updateConfigSocket(event.target.value)}
                  placeholder="/tmp/mysql.sock"
                />
              </label>
            </div>
            <button className="smallAction configOpenAction" type="button" onClick={openMysqlConfigDocument}>
              Open my.cnf
            </button>
            <div className="modalActions">
              <button className="smallAction" type="button" onClick={cancelConfigure}>
                Cancel
              </button>
              <button className="smallAction primary" type="button" onClick={saveMysqlConfig}>
                Save config
              </button>
            </div>
          </div>
        </div>
      )}

      {confirmRemove && (
        <div className="modalOverlay" onMouseDown={() => setConfirmRemove(null)}>
          <div
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="confirm-title"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <h3 id="confirm-title" className="modalTitle">
              Remove {confirmRemove.name}?
            </h3>
            <p className="modalBody">
              {confirmRemove.managed
                ? "This will stop the service and permanently delete its data directory."
                : "This will remove the service from the list. The underlying service itself stays untouched."}
            </p>
            <div className="modalActions">
              <button
                className="smallAction"
                type="button"
                onClick={() => setConfirmRemove(null)}
              >
                Cancel
              </button>
              <button
                className="smallAction danger"
                type="button"
                onClick={async () => {
                  const target = confirmRemove;
                  setConfirmRemove(null);
                  await removeService(target.id);
                }}
              >
                Remove
              </button>
            </div>
          </div>
        </div>
      )}
    </main>
  );
}

export default App;
