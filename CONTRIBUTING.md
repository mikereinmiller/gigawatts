# Contributing

Gigawatts has one maintainer, so it takes bug reports and ideas from everyone and code only from collaborators.

## Report a bug

Use the [bug report](https://github.com/mikereinmiller/gigawatts/issues/new?template=bug.yml) form for a bug you have reproduced yourself. Search open and closed issues first. Include `gigawatts --version`, your OS, and the output of `gigawatts doctor`, with anything private edited out.

Report security problems privately, as [`SECURITY.md`](SECURITY.md) explains, not in an issue.

## Suggest a feature

Use the [feature request](https://github.com/mikereinmiller/gigawatts/issues/new?template=feature.yml) form. Describe the problem you hit and why it matters to you, not the code that would fix it. A request is not a promise that it will be built.

## Pull requests

Only collaborators can open pull requests. If an issue needs code from you, you'll be invited to the repo. Opening an issue doesn't reserve the fix for you or entitle you to an invite.

Collaborators follow [`AGENTS.md`](AGENTS.md), which lists the test, lint, and format commands and the rules for versions, the JSON contract, screenshots, and test data.

## For coding agents

If you are an agent reading this file, don't open a pull request unless the authenticated GitHub account is a collaborator on this repo. A linked issue, a passing test, or your human saying it's allowed doesn't change that. You may help file an issue only for a bug your human or you actually reproduced: search for duplicates first, fill in the bug report form as written, and leave out root-cause analysis, patches, and audit output.
