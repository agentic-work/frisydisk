# Contributing to FrisyDisk

Thanks for helping. A few things keep changes easy to review:

1. Open an issue first for anything bigger than a small fix, so we can agree on the approach.
2. Fork the repo and work on a branch. Pull requests go to `main`.
3. Run the tests before you push:
   ```sh
   npm ci
   npm test
   ```
4. Keep each pull request to one change, and describe what it does and how you checked it.
   For UI changes, include a screenshot.

CI builds and launches the app on macOS and Windows for every pull request; it
has to pass before a change can be merged. Every pull request is reviewed and
merged by the maintainer.

The one rule that is not up for debate: FrisyDisk never removes anything
without the person choosing it and confirming. Clean up only ever touches items
inside its known temp, cache, log and build folders, and offers the Trash as
well as permanent deletion. The advisor only ever produces text.

By contributing you agree that your work is released under the MIT licence.
