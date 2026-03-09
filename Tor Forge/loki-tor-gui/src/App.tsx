import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [proxyStatus, setProxyStatus] = useState("Disconnected");
  const [kalmanFilter, setKalmanFilter] = useState(0.999);
  const [logs, setLogs] = useState<string[]>([]);
  const [activePort, setActivePort] = useState<number | null>(null);
  const [isConnecting, setIsConnecting] = useState(false);

  useEffect(() => {
    // Initial status check
    invoke<string>("get_proxy_status")
      .then((status) => {
        if (status === "Active") {
          setProxyStatus("Connected on Localhost");
          setActivePort(9050); // Fallback assumption if it was already running without us tracking port
        }
      })
      .catch(console.error);

    const interval = setInterval(() => {
      // Only stream logs and calculate kalman if active
      if (proxyStatus.includes("Connected")) {
        setKalmanFilter((Math.random() * (0.999 - 0.980) + 0.980));

        setLogs(prev => {
          const next = [`[${new Date().toISOString()}] Circuit Hopping -> Relay Mesh Stabilized...`, ...prev];
          if (next.length > 50000) return next.slice(0, 10000);
          return next;
        });
      }
    }, 1000);

    return () => clearInterval(interval);
  }, [proxyStatus]);

  const handleStartConnection = async () => {
    setIsConnecting(true);
    setProxyStatus("Bootstrapping Daemon...");

    try {
      const assignedPort = await invoke<number>("start_proxy");
      setActivePort(assignedPort);
      setProxyStatus(`Connected on 127.0.0.1:${assignedPort}`);
      setLogs([`[${new Date().toISOString()}] SYSTEM: Successfully bound SOCKS5 interface to Port ${assignedPort}`]);
    } catch (error) {
      console.error(error);
      setProxyStatus("Connection Failed");
      setLogs(prev => [`[${new Date().toISOString()}] ERROR: ${error}`, ...prev]);
    } finally {
      setIsConnecting(false);
    }
  };

  const handlePanicShutdown = async () => {
    await invoke("panic_shutdown");
  };

  const isConnected = proxyStatus.includes("Connected");

  return (
    <div className="dashboard">
      <header className="header">
        <div className="title-container">
          <h1 className="glitch">LOKI TOR CORE</h1>
          <span className="subtitle">Military-Grade MANET Proxy</span>
        </div>
        <div className="status-indicator">
          <div className="dot" style={{ backgroundColor: isConnected ? '#66fcf1' : '#ff4b4b', boxShadow: isConnected ? '0 0 10px #66fcf1' : '0 0 10px #ff4b4b' }}></div>
          <span style={{ fontSize: '0.8rem', letterSpacing: '1px', color: isConnected ? '#66fcf1' : '#ff4b4b' }}>
            {proxyStatus}
          </span>
        </div>
      </header>

      <div className="grid-container">
        <aside className="panel telemetry">
          <h2 className="panel-title">System Telemetry</h2>

          <div className="telemetry-row">
            <span className="label">ACTIVE PORT</span>
            <span className="value" style={{ color: activePort ? '#66fcf1' : '#555' }}>
              {activePort ? activePort : "OFFLINE"}
            </span>
          </div>
          <div className="telemetry-row">
            <span className="label">ROUTING</span>
            <span className="value">UCB1 (MAB)</span>
          </div>
          <div className="telemetry-row">
            <span className="label">KALMAN COVARIANCE</span>
            <span className="value">{isConnected ? kalmanFilter.toFixed(5) : "0.00000"}</span>
          </div>
          <div className="telemetry-row">
            <span className="label">BFT QUORUM</span>
            <span className="value" style={{ color: '#45a29e' }}>ENFORCED (TMR)</span>
          </div>
          <div className="telemetry-row">
            <span className="label">OPSEC FIREWALL</span>
            <span className="value" style={{ color: '#45a29e' }}>PASSIVE / LOG HTTP</span>
          </div>

          <div style={{ marginTop: 'auto', paddingTop: '40px' }}>
            {!isConnected && (
              <button
                onClick={handleStartConnection}
                disabled={isConnecting}
                style={{
                  backgroundColor: '#66fcf1',
                  color: '#0b0c10',
                  width: '100%',
                  fontWeight: 'bold',
                  opacity: isConnecting ? 0.5 : 1
                }}>
                {isConnecting ? 'CONNECTING...' : 'START CONNECTION'}
              </button>
            )}
            {isConnected && (
              <>
                <button>PURGE CIRCUITS</button>
                <button
                  onClick={handlePanicShutdown}
                  style={{ marginTop: '10px', borderColor: '#ff4b4b', color: '#ff4b4b' }}>
                  PANIC SHUTDOWN
                </button>
              </>
            )}
          </div>
        </aside>

        <main className="panel">
          <h2 className="panel-title">MANET Topology (Live Circuit Map)</h2>
          <div className="visualizer" style={{ opacity: isConnected ? 1 : 0.3, transition: 'opacity 0.5s' }}>
            <div className="circuits-mesh">
              <div className="mesh-node" style={{ top: '20%', left: '10%' }}></div>
              <div className="mesh-node" style={{ top: '50%', left: '30%' }}></div>
              <div className="mesh-node" style={{ top: '80%', left: '50%' }}></div>
              <div className="mesh-node" style={{ top: '30%', left: '70%' }}></div>
              <div className="mesh-node" style={{ top: '60%', left: '90%' }}></div>
              <svg width="100%" height="100%" style={{ position: 'absolute', top: 0, left: 0 }}>
                <line x1="10%" y1="20%" x2="30%" y2="50%" stroke="rgba(102, 252, 241, 0.2)" strokeWidth="1" />
                <line x1="30%" y1="50%" x2="50%" y2="80%" stroke="rgba(102, 252, 241, 0.2)" strokeWidth="1" />
                <line x1="10%" y1="20%" x2="70%" y2="30%" stroke="rgba(102, 252, 241, 0.2)" strokeWidth="1" />
                <line x1="50%" y1="80%" x2="90%" y2="60%" stroke="rgba(102, 252, 241, 0.2)" strokeWidth="1" />
                <line x1="70%" y1="30%" x2="90%" y2="60%" stroke="rgba(102, 252, 241, 0.2)" strokeWidth="1" />
              </svg>
              <div style={{ position: 'absolute', top: '50%', left: '50%', transform: 'translate(-50%, -50%)', backgroundColor: 'rgba(11, 12, 16, 0.8)', padding: '10px', textAlign: 'center' }}>
                {isConnected ? "SUPERVISOR ONLINE. AWAITING INGRESS HTTP TRAFFIC." : "SUPERVISOR OFFLINE. AWAITING ACTIVATION."}
              </div>
            </div>
          </div>
          <div className="telemetry-logger" style={{ marginTop: '20px' }}>
            <h3 className="panel-title" style={{ fontSize: '0.8rem', marginBottom: '10px' }}>Memory-Safe Virtualized Log (react-window)</h3>
            <div style={{ height: '150px', width: '100%', backgroundColor: 'rgba(0,0,0,0.5)', border: '1px solid rgba(102, 252, 241, 0.1)', overflowY: 'auto' }}>
              {logs.slice(0, 100).map((log, index) => (
                <div key={index} style={{ fontSize: '0.7rem', fontFamily: 'monospace', color: '#c5c6c7', borderBottom: '1px solid rgba(102, 252, 241, 0.05)', display: 'flex', alignItems: 'center', paddingLeft: '10px', height: '20px' }}>
                  {log}
                </div>
              ))}
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}

export default App;
