export default function DashboardLoading() {
  return (
    <div className="skeleton-shell">
      {/* Sidebar skeleton */}
      <div className="skeleton-sidebar">
        <div className="skeleton-sidebar-header">
          <div className="skeleton-line" style={{ width: "60%", height: 18 }} />
          <div className="skeleton-circle" style={{ width: 28, height: 28 }} />
        </div>
        <div className="skeleton-sidebar-search">
          <div className="skeleton-line" style={{ width: "100%", height: 34, borderRadius: 6 }} />
        </div>
        <div className="skeleton-sidebar-list">
          {Array.from({ length: 8 }).map((_, i) => (
            <div key={i} className="skeleton-conv-item">
              <div className="skeleton-circle" style={{ width: 40, height: 40 }} />
              <div className="skeleton-conv-text">
                <div className="skeleton-line" style={{ width: `${60 + (i % 3) * 12}%`, height: 13 }} />
                <div className="skeleton-line" style={{ width: `${40 + (i % 4) * 10}%`, height: 11, opacity: 0.5 }} />
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Main area skeleton */}
      <div className="skeleton-main">
        <div className="skeleton-header">
          <div className="skeleton-line" style={{ width: 160, height: 18 }} />
          <div className="skeleton-line" style={{ width: 100, height: 12, opacity: 0.5 }} />
        </div>
        <div className="skeleton-messages">
          {Array.from({ length: 5 }).map((_, i) => (
            <div key={i} className={`skeleton-msg ${i % 3 === 0 ? "skeleton-msg-self" : ""}`}>
              {/* Own messages render without an avatar column. */}
              {i % 3 !== 0 && <div className="skeleton-circle" style={{ width: 40, height: 40 }} />}
              <div className="skeleton-msg-body">
                <div className="skeleton-line" style={{ width: 80, height: 11 }} />
                <div
                  className="skeleton-line"
                  style={{
                    width: `${30 + ((i * 17) % 50)}%`,
                    height: 40 + ((i * 13) % 30),
                    borderRadius: 8,
                  }}
                />
              </div>
            </div>
          ))}
        </div>
        <div className="skeleton-input">
          <div className="skeleton-line" style={{ width: "100%", height: 44, borderRadius: 8 }} />
        </div>
      </div>
    </div>
  );
}
