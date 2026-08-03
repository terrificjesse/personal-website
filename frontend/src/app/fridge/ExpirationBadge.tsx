function daysUntil(dateIso: string): number {
  const ms = new Date(dateIso).getTime() - Date.now();
  return Math.ceil(ms / (1000 * 60 * 60 * 24));
}

export function ExpirationBadge({ estimatedExpiration }: { estimatedExpiration: string | null }) {
  if (!estimatedExpiration) {
    return (
      <span className="rounded-full bg-gray-100 px-2.5 py-0.5 text-xs text-gray-600 dark:bg-gray-800 dark:text-gray-300">
        no estimate
      </span>
    );
  }

  const days = daysUntil(estimatedExpiration);

  let colorClasses = "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200";
  let label = `${days}d left`;
  if (days < 0) {
    colorClasses = "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200";
    label = "expired";
  } else if (days <= 2) {
    colorClasses = "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200";
  } else if (days <= 5) {
    colorClasses = "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200";
  }

  return (
    <span className={`rounded-full px-2.5 py-0.5 text-xs font-medium ${colorClasses}`}>{label}</span>
  );
}
