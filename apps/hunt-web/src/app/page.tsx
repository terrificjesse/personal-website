import { redirect } from "next/navigation";

/**
 * The listings are the site. They stay at `/internships` so the extension's links and the Gmail
 * redirect, written when this was a tab, keep working unchanged.
 */
export default function Home() {
  redirect("/internships");
}
