import { useEffect, useState } from 'react';
import { Link, useParams } from '../router';
import { fetchGroup } from '../api';
import type { GroupResponse } from '../api';

export function Group() {
  const { groupPath = '' } = useParams<{ groupPath: string }>();
  const [group, setGroup] = useState<GroupResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setGroup(null);
    setError(null);
    fetchGroup(groupPath).then(setGroup).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : 'Could not load this group.');
    });
  }, [groupPath]);

  if (error) return <p role="alert">{error}</p>;
  if (!group) return <p aria-live="polite">Loading group...</p>;

  return (
    <main>
      <p><Link to="/">Overview</Link></p>
      <h1>{group.group_path}</h1>
      {group.targets.length === 0 ? (
        <p>No active targets are in this group.</p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0 }}>
          {group.targets.map((target) => (
            <li
              key={target.target_id}
              style={{
                padding: 12,
                marginBottom: 8,
                border: '1px solid var(--border)',
                borderRadius: 6,
                background: 'var(--bg-card)',
              }}
            >
              <Link to={`/targets/${encodeURIComponent(target.target_id)}`}>
                {target.name}
              </Link>
              <span style={{ marginLeft: 8, color: 'var(--text-muted)' }}>
                {target.state}, {target.checks_count} checks
              </span>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
