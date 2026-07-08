# Operation Wizard — 설계문서 (Design Docs)

이 디렉터리는 Operation Wizard 프로젝트의 **설계 기준 문서**입니다.
구현 세부보다는 **컨셉·구조·주요 결정사항**을 다룹니다.

> ⚠️ 이 문서들은 프로젝트 루트의 `CLAUDE.md`에서 `@` 임포트되어
> **모든 작업/질문 시 자동으로 컨텍스트에 포함**됩니다.
> 별도 언급이 없어도 항상 이 문서를 기준으로 답하고 작업합니다.

## 문서 구성

| 파일 | 내용 |
|------|------|
| [01-overview.md](01-overview.md) | 프로젝트 개요, 목표, 범위, 용어 |
| [02-architecture.md](02-architecture.md) | 전체 아키텍처 (Tauri 2 / 백엔드·프론트 구조 / IPC / 데이터 흐름) |
| [03-agent-detection.md](03-agent-detection.md) | 핵심 기능: 로컬 CLI 에이전트 탐지 컨셉 |
| [04-ui-and-design-system.md](04-ui-and-design-system.md) | UI 셸 + Open Design 디자인 시스템 |
| [05-decisions.md](05-decisions.md) | 주요 결정 로그 (결정·근거·대안) |
| [06-build-and-environment.md](06-build-and-environment.md) | 빌드/실행 환경 제약 (MSVC, 사내망) |
| [07-workspace-and-runs.md](07-workspace-and-runs.md) | 에이전트 실행(run) 엔진 + 대화/캔버스 워크스페이스 |
| [08-guided-flows-and-skills.md](08-guided-flows-and-skills.md) | 가이드 플로우(옵션 우선 시작·프리필) + 시스템 스킬 + 결과형식 강제 가이드 |

## 유지보수 규칙 (중요)

1. **설계가 바뀌면 코드와 함께 이 문서도 같은 작업 단위에서 업데이트한다.**
   (새 기능·모듈 추가, 데이터 모델 변경, 주요 기술 선택 변경, 빌드 절차 변경 등)
2. 변경의 **이유**가 있으면 [05-decisions.md](05-decisions.md)에 결정 항목으로 남긴다.
3. 컨셉 수준을 유지한다. 라인 단위 구현 디테일은 코드와 주석/`README.md`에 둔다.
4. 한 문서가 과도하게 길어지면 분할하고, 이 인덱스 표와 `CLAUDE.md`의 임포트 목록을 갱신한다.
5. 문서 간 참조는 상대 경로 링크로 연결한다.

## 관련 문서

- 루트 [`README.md`](../../README.md) — 빌드/실행/테스트 등 개발자용 실행 가이드
- 원본 참조: Open Design 저장소 `docs/cli-agent-detection-and-daemon.ko.md`
