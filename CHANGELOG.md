# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/fa-sharp/rocket-flex-session/compare/v0.1.3...v0.2.0) - 2025-09-10

### Added

- [**breaking**] add sqlite storage, refactor storage traits and inner session ([#6](https://github.com/fa-sharp/rocket-flex-session/pull/6))
- use builder pattern ([#5](https://github.com/fa-sharp/rocket-flex-session/pull/5))
- session indexing ([#1](https://github.com/fa-sharp/rocket-flex-session/pull/1))

### Fixed

- indexed storages missing impl
- unnecessary cookie creation when updating the session data ([#2](https://github.com/fa-sharp/rocket-flex-session/pull/2))

### Other

- Update README.md
- bump rand & retainer, use rocket exports for deps ([#3](https://github.com/fa-sharp/rocket-flex-session/pull/3))
