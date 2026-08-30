/*
 * The content script: reads labels, fills what it may, reports what it did.
 *
 * Loaded on the known ATS hosts only. **It does nothing on load** — it registers a listener
 * and waits. Filling happens when you press the button in the popup, which is rule 10's
 * "explicit user action, never on page load": a script that fills automatically writes your
 * real name and phone number into the DOM of anything matching the pattern, including a
 * phishing clone of a careers page.
 *
 * It also never submits. It fills and stops; you review and send. A misfilled application sent
 * automatically cannot be recalled.
 *
 * The matching logic lives in `fields.js` as pure functions, imported here at fill time.
 */

/**
 * The matching logic, injected alongside this file as a plain script.
 *
 * Not an `import()`: a module would need `web_accessible_resources`, and for the activeTab
 * path that means declaring it reachable from every page — exposure bought for nothing when
 * two scripts injected together need no declaration at all.
 */
function fields() {
  const module = globalThis.HuntFields;
  if (!module) throw new Error("hunt: fields.js was not injected alongside fill.js");
  return module;
}

/**
 * The visible label for a control, tried in order of how much a human would trust it.
 *
 * `name` and `id` come last and are the only non-visible sources: on a well-built form they
 * are redundant, and on a badly-built one they are all there is.
 */
function labelFor(element) {
  if (element.id) {
    const explicit = document.querySelector(`label[for="${CSS.escape(element.id)}"]`);
    if (explicit?.textContent?.trim()) return explicit.textContent;
  }

  const wrapping = element.closest("label");
  if (wrapping?.textContent?.trim()) return wrapping.textContent;

  const describedBy = element.getAttribute("aria-labelledby");
  if (describedBy) {
    const text = describedBy
      .split(/\s+/)
      .map((id) => document.getElementById(id)?.textContent || "")
      .join(" ")
      .trim();
    if (text) return text;
  }

  const aria = element.getAttribute("aria-label");
  if (aria?.trim()) return aria;

  const placeholder = element.getAttribute("placeholder");
  if (placeholder?.trim()) return placeholder;

  return `${element.getAttribute("name") || ""} ${element.id || ""}`;
}

/**
 * Set a value the way React will believe.
 *
 * Assigning `element.value` directly does not register with a React-controlled input: the
 * framework's state never updates and the value is wiped on the next render. Going through the
 * prototype's native setter and then dispatching bubbling `input` and `change` events is what
 * makes the change look like typing. This is the first bug anyone hits here, per
 * `apps/hunt-extension/CLAUDE.md`, and it fails *later* — the field looks filled until
 * something else re-renders.
 */
function setNativeValue(element, value) {
  const prototype =
    element instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : element instanceof HTMLSelectElement
        ? HTMLSelectElement.prototype
        : HTMLInputElement.prototype;

  const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
  if (setter) {
    setter.call(element, value);
  } else {
    element.value = value;
  }

  element.dispatchEvent(new Event("input", { bubbles: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
}

/** The string to type for a profile value, or null if there is nothing to type. */
function renderValue(key, profile) {
  const value = profile[key];
  if (value === null || value === undefined) return null;
  if (key === "needs_sponsorship") return value ? "Yes" : "No";
  return String(value);
}

/**
 * Choose an option whose visible text matches, for a `<select>`.
 *
 * Exact-ish only: a select is a closed set of answers, and picking the "closest" one on a
 * sponsorship or work-authorization question is how you assert something untrue about
 * yourself. No match means the field is left alone.
 */
function selectOption(element, wanted, normalize) {
  const target = normalize(wanted);
  for (const option of element.options) {
    if (normalize(option.textContent) === target || normalize(option.value) === target) {
      return option.value;
    }
  }
  return null;
}

/** Every control we might consider. Radios and checkboxes are deliberately absent — see fill. */
function candidates() {
  return [...document.querySelectorAll("input, textarea, select")].filter((element) => {
    if (element.disabled || element.readOnly) return false;
    const type = (element.getAttribute("type") || "text").toLowerCase();
    return !["radio", "checkbox", "submit", "button", "reset", "image", "range", "color"].includes(
      type,
    );
  });
}

/**
 * Fill what we can. Returns a report; writes nothing else and clicks nothing.
 *
 * Skips controls that already hold a value. Overwriting something you typed would be the one
 * unrecoverable thing a fill can do, and "it was already right" is the common case.
 */
function fill(profile) {
  const { classify, inputIsBlocked, normalizeLabel } = fields();

  const report = { filled: [], blocked: [], alreadyFilled: 0, unmatched: 0 };

  for (const element of candidates()) {
    const label = labelFor(element);

    // Attribute check and label check are independent — a password field can sit under an
    // innocuous label, and a card-number label can sit on a plain text input.
    if (
      inputIsBlocked({
        type: element.getAttribute("type"),
        autocomplete: element.getAttribute("autocomplete"),
        name: element.getAttribute("name"),
        id: element.id,
      })
    ) {
      report.blocked.push({ label: normalizeLabel(label), reason: "sensitive" });
      continue;
    }

    const verdict = classify(label);
    if (verdict.kind === "blocked") {
      report.blocked.push({ label: normalizeLabel(label), reason: verdict.reason });
      continue;
    }
    if (verdict.kind === "skip") {
      report.unmatched += 1;
      continue;
    }

    if (element.value && String(element.value).trim() !== "") {
      report.alreadyFilled += 1;
      continue;
    }

    const wanted = renderValue(verdict.key, profile);
    if (wanted === null) {
      report.unmatched += 1;
      continue;
    }

    if (element instanceof HTMLSelectElement) {
      const option = selectOption(element, wanted, normalizeLabel);
      if (option === null) {
        report.unmatched += 1;
        continue;
      }
      setNativeValue(element, option);
    } else {
      setNativeValue(element, wanted);
    }

    report.filled.push({ label: normalizeLabel(label), key: verdict.key });
  }

  return report;
}

// The only entry point. Nothing above runs until this message arrives, and the message only
// comes from the popup's button.
browser.runtime.onMessage.addListener((message) => {
  if (message?.type === "hunt-fill") {
    return Promise.resolve(fill(message.profile || {}));
  }
  if (message?.type === "hunt-ping") {
    return Promise.resolve({ ready: true, host: location.host });
  }
  return undefined;
});
