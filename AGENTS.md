## Development Workflow

**IMPORTANT**: After any code change, bug fix, or feature addition/removal, you MUST complete all of these steps:

1. **Update README.md** if the change affects:
   - Usage examples or commands
   - Installation instructions
   - Configuration options
   - Available features

2. **Update CHANGELOG.md**:
   - Add new version number following semantic versioning (MAJOR.MINOR.PATCH)
   - Add entry under appropriate category (Added, Changed, Fixed, Removed)
   - Include date in format YYYY-MM-DD

3. **Update docs/improvements-and-features.md**:
   - When a listed feature or improvement is 100% finished, append ` - DONE` to its heading
   - Do not mark partially completed work as done

4. **Commit and push changes**:
   - Use `git add` to stage all modified files (README.md, CHANGELOG.md, and code files)
   - Create descriptive commit message following existing style
   - Push to remote repository with `git push`
