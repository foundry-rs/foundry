# Contributing to Kiro

Thank you for your interest in contributing to Kiro! This guide will help you get started.

## Code of Conduct

By participating in this project, you agree to abide by our Code of Conduct. Please be respectful and constructive in all interactions.

## Getting Started

### Prerequisites

- Node.js 18+ and npm 9+
- Git
- A code editor (we recommend VS Code)

### Setting Up Your Development Environment

1. **Fork the repository** on GitHub

2. **Clone your fork:**
bash
   git clone https://github.com/YOUR_USERNAME/kiro.git
   cd kiro


3. **Add the upstream repository:**
bash
   git remote add upstream https://github.com/kiro-ai/kiro.git


4. **Install dependencies:**
bash
   npm install


5. **Create a branch for your work:**
bash
   git checkout -b feature/your-feature-name


## Development Workflow

### Running Tests

bash
# Run all tests
npm test

# Run tests in watch mode
npm test -- --watch

# Run tests with coverage
npm test -- --coverage


### Linting and Formatting

bash
# Check code style
npm run lint

# Auto-fix linting issues
npm run lint:fix

# Format code
npm run format


### Building

bash
# Build the project
npm run build

# Build in watch mode
npm run build:watch


## Making Changes

### Commit Messages

We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

- `feat:` New features
- `fix:` Bug fixes
- `docs:` Documentation changes
- `style:` Code style changes (formatting, missing semicolons, etc.)
- `refactor:` Code refactoring
- `test:` Adding or updating tests
- `chore:` Maintenance tasks

**Examples:**

feat: add support for TypeScript 5.0
fix: resolve memory leak in file watcher
docs: update installation instructions


### Pull Request Process

1. **Ensure your code passes all checks:**
bash
   npm run lint
   npm test
   npm run build


2. **Update documentation** if you've changed APIs or added features

3. **Write or update tests** for your changes

4. **Keep your branch up to date:**
bash
   git fetch upstream
   git rebase upstream/main


5. **Push your changes:**
bash
   git push origin feature/your-feature-name


6. **Create a Pull Request** on GitHub with:
   - Clear title following commit message conventions
   - Description of what changed and why
   - Reference to any related issues
   - Screenshots or examples if applicable

7. **Address review feedback** promptly and professionally

## Code Style Guidelines

- Use TypeScript for all new code
- Follow the existing code style (enforced by ESLint and Prettier)
- Write clear, self-documenting code with comments for complex logic
- Keep functions small and focused
- Use meaningful variable and function names

## Testing Guidelines

- Write unit tests for new features and bug fixes
- Aim for high test coverage, especially for critical paths
- Use descriptive test names that explain what is being tested
- Mock external dependencies appropriately

## Documentation

- Update README.md if you change user-facing functionality
- Add JSDoc comments for public APIs
- Update CHANGELOG.md following [Keep a Changelog](https://keepachangelog.com/) format

## Reporting Issues

When reporting bugs, please include:

- Clear description of the issue
- Steps to reproduce
- Expected vs actual behavior
- Environment details (OS, Node version, etc.)
- Relevant logs or error messages

## Questions?

If you have questions or need help:

- Check existing issues and discussions
- Open a new discussion on GitHub
- Reach out to maintainers

Thank you for contributing to Kiro!
