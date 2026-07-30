import { useEffect, useState } from 'react';
import { Link, useParams } from '../router';
import { fetchGroup, fetchSeries, fetchTarget } from '../api';
import type { GroupResponse } from '../api';
import { useLiveRefresh } from '../live';
import {
  ComparisonGraph,
  type ComparisonCheck,
} from '../components/ComparisonGraph';

export function Group() {
  const { groupPath = '' } = useParams<{ groupPath: string }>();
  const [group, setGroup] = useState<GroupResponse | null>(null);
  const [comparisonChecks, setComparisonChecks] = useState<ComparisonCheck[]>([]);
  const [error, setError] = useState<string | null>(null);
  const liveRevision = useLiveRefresh();

  useEffect(() => {
    let cancelled = false;
    setGroup(null);
    setComparisonChecks([]);
    setError(null);
    fetchGroup(groupPath)
      .then(async (result) => {
        if (cancelled) return;
        setGroup(result);
        const details = await Promise.all(
          result.targets.map((target) => fetchTarget(target.target_id)),
        );
        if (cancelled) return;
        setComparisonChecks(
          details.flatMap((detail) => {
            const check = detail.checks[0];
            if (!check) return [];
            return [
              {
                targetId: detail.target_id,
                checkId: check.check_id,
                label: `${detail.name || detail.target_id} / ${check.check_id}`,
              },
            ];
          }),
        );
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(
            reason instanceof Error ? reason.message : 'Could not load this group.',
          );
        }
      });
    return () => {
      cancelled = true;
    };
  }, [groupPath, liveRevision]);

  if (error) return <p role="alert">{error}</p>;
  if (!group) return <p aria-live="polite">Loading group...</p>;

  return (
    <main>
      <p><Link to="/">Overview</Link></p>
      <h1>{group.group_path}</h1>
      {comparisonChecks.length >= 2 && (
        <div style={{ marginBottom: 24 }}>
          <ComparisonGraph
            checks={comparisonChecks}
            fetchSeries={fetchSeries}
          />
          <p style={{ color: 'var(--text-muted)', fontSize: 12 }}>
            The graph compares the first active check from each target in this group.
          </p>
        </div>
      )}
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
