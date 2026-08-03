import Link from "next/link";

export default function Home() {
  return (
    <div className="mx-auto max-w-4xl px-4 py-16">
      <h1 className="text-2xl font-semibold">Personal Website</h1>
      <p className="mt-2 opacity-70">A handful of small self-contained tabs live here.</p>
      <ul className="mt-8 space-y-2">
        <li>
          <Link href="/fridge" className="underline underline-offset-4">
            Fridge
          </Link>
        </li>
      </ul>
    </div>
  );
}
