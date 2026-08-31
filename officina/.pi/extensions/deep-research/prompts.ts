// Prompt templates for the deep-research reasoning + research steps.
//
// Every reasoning step (clarify, brief, decompose, gap-analysis) is a child
// little-coder — an extension can't call inference directly — so these are the
// task strings handed to runSubCoder. JSON-emitting prompts say "ONLY a JSON
// array … no prose" because the reply is parsed by extractJsonArray, which is
// tolerant of fences but happier with clean output.
//
// The guidance mirrors the deep-research reference: clarify up front (maximize
// relevance, don't invent preferences, avoid overasking), compress into a single
// research brief that becomes the north star, scale research to the brief, and
// write ONE cited markdown report from the brief + all findings.

// Prepended to every reasoning step. These steps PLAN the research — they must
// not browse or answer the topic, just emit their instruction/JSON directly, so
// they stay fast and don't fabricate sources.
const NO_TOOLS = "Do not use any tools and do not browse — answer directly from what is given. ";

// Phase 1a — clarifying questions (Scope). Kept to 1-4, each with suggested
// answers so the user can pick fast.
export function clarifyPrompt(topic: string): string {
  return (
    NO_TOOLS +
    `You are SCOPING a research task, not answering it. Read the topic and propose the ` +
    `1-4 clarifying questions whose answers would most change the shape of the final report ` +
    `(scope, timeframe, audience, depth, comparison axes, constraints). Only ask what is ` +
    `genuinely necessary — do NOT invent preferences the user never implied, and do not ask ` +
    `more than 4. For each question give 1-3 short suggested answers.\n\n` +
    `Output ONLY a JSON array of {"q": "<question>", "options": ["<short answer>", ...]}. No prose.\n\n` +
    `Topic:\n${topic}`
  );
}

// Phase 1b — compress topic + answers into the research brief (the north star).
// Returns prose, not JSON.
export function briefPrompt(topic: string, answers: string): string {
  return (
    NO_TOOLS +
    `Your entire reply must BE the brief — a scoping instruction to the research team, NOT an ` +
    `answer to the topic and NOT any findings, statistics, or citations. ` +
    `Write a concise RESEARCH BRIEF that will steer a team of research agents. Fold in every ` +
    `detail from the topic and the user's answers; where a dimension is unstated, leave it ` +
    `open-ended rather than assuming. State clearly: the core question, the sub-questions to ` +
    `cover, the scope/timeframe, and what the final report must deliver. Write in the first ` +
    `person as if instructing the team. Keep it under ~250 words. Output the brief only — no ` +
    `preamble, no questions back.\n\n` +
    `Topic:\n${topic}\n\nUser's answers to clarifying questions:\n${answers}`
  );
}

// Phase 2 — the lead splits the brief into `count` independent research subtopics.
export function decomposePrompt(brief: string, count: number): string {
  return (
    NO_TOOLS +
    `You are the LEAD researcher. Split this brief into exactly ${count} INDEPENDENT subtopics ` +
    `that can be researched in parallel with no overlap — together they must cover the brief. ` +
    `For each, write a specific research instruction for an agent that can browse the web and ` +
    `read local files.\n\n` +
    `Output ONLY a JSON array of ${count} objects {"label": "<3-4 word name>", ` +
    `"task": "<specific research instruction>"}. No prose.\n\n` +
    `Research brief:\n${brief}`
  );
}

// Phase 3 — after wave 1, decide whether gaps remain worth up to `maxGap` more
// agents. Returning [] is the expected, common answer — don't spawn to fill quota.
export function gapPrompt(brief: string, digest: string, maxGap: number): string {
  return (
    NO_TOOLS +
    `You are the LEAD researcher reviewing what wave-1 agents found against the brief. ` +
    `Identify only GENUINE gaps or unresolved contradictions that materially affect the report ` +
    `and would benefit from targeted follow-up research. Propose AT MOST ${maxGap} follow-up ` +
    `tasks — fewer is better, and an empty array is correct if coverage is already sufficient. ` +
    `Do NOT invent tasks just to use the budget.\n\n` +
    `Output ONLY a JSON array (0 to ${maxGap}) of {"label": "<3-4 word name>", ` +
    `"task": "<specific follow-up research instruction>"}. No prose.\n\n` +
    `Research brief:\n${brief}\n\nWave-1 findings:\n${digest}`
  );
}

// The single-agent path (M=1): one agent researches the whole brief itself.
export function singleAgentTask(brief: string): string {
  return (
    `Research this brief thoroughly using web search/browsing. For every factual claim, record the FULL ` +
    `source URL you got it from, and only state what you actually found in your sources — do not guess or ` +
    `fabricate names, numbers, or versions.\n\nResearch brief:\n${brief}`
  );
}

// Phase 4 — the block injected into the main agent's system prompt for the
// one-shot write. The main agent (not a sub-coder) writes it, so it isn't capped
// at the 2000-char sub-coder report limit and can produce a full report.
export function writeInstructionBlock(
  topic: string,
  brief: string,
  digest: string,
  failedCount = 0,
): string {
  // When research agents were lost (watchdog kill after the retry, or a hard
  // failure), their subtopic is simply missing from the findings. Tell the writer
  // to disclose that coverage gap rather than paper over it with invented content —
  // the eval showed the model will otherwise quietly write a thinner report that
  // reads as complete.
  const coverageNote =
    failedCount > 0
      ? `- IMPORTANT — partial coverage: ${failedCount} research ${failedCount === 1 ? "agent" : "agents"} ` +
        `returned no findings (their sections are marked "(research failed…)" below). Add a short ` +
        `"### Research coverage" note near the end stating that ${failedCount} ` +
        `${failedCount === 1 ? "subtopic was" : "subtopics were"} not covered and the report's scope is ` +
        `limited accordingly. Do NOT invent content to fill the gap.\n`
      : "";
  return (
    `\n\n## Deep Research — write the report\n` +
    `The user's message is a research topic. Do NOT do more research and do NOT call any tools. ` +
    `Using ONLY the research brief and the findings below, write a single, cohesive markdown ` +
    `report as your reply, in one shot.\n\n` +
    `Report rules:\n` +
    coverageNote +
    `- Start with an H1 title and a 2-4 sentence executive summary.\n` +
    `- Use ## / ### headings; prefer prose over bullet lists; include a comparison table where useful.\n` +
    `- Write in a single neutral voice — no "I found", "the agents", or other self-reference.\n` +
    `- Use ONLY facts stated in the findings below. Do NOT add outside knowledge or invent any specifics ` +
    `(names, version numbers, dates, benchmark figures). If the findings don't support a claim, omit it or ` +
    `say the data was not found — never guess.\n` +
    `- Number citations inline as [1], [2] …, one number per unique SOURCE URL from the findings.\n` +
    `- End with a "### Sources" section mapping each number to the actual source URL taken from the findings. ` +
    `Do NOT fabricate URLs; if a finding gave no URL for a claim, mark that claim "(source not provided)" rather ` +
    `than inventing one, and only list real URLs under Sources.\n` +
    `- Cover every dimension the brief asks for; where the findings conflict or fall short, say so plainly.\n\n` +
    `### Research brief\n${brief}\n\n` +
    `### Findings\n${digest}\n\n` +
    `### Topic (for reference)\n${topic}`
  );
}
