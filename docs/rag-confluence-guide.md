# RAG + Confluence 연동 가이드

이 문서는 Operation Wizard의 **사내 문서 RAG 검색**과 **Confluence 수집** 기능을 **처음부터 실제로
동작**시키기 위한 실무 가이드다. 개념·설계 근거는 `docs/design/`(특히
[05-decisions.md](design/05-decisions.md) D46/D48/D50/D51/D53/D59/D63/D65)에 있고, 이 문서는 **어디에
무엇을 입력하고, 안 될 때 무엇을 확인하는지**만 다룬다.

> 대상: 이 앱을 사내에서 운영·배포하는 담당자. **RAG 검색은 지식 화면 입력(C절)만으로 동작**하고,
> A절(코드 조정)은 사내 Fabrix 계약이 기본과 다를 때만 필요하다. 나머지(B~F)는 앱 안에서 클릭으로 끝난다.

---

## 0. 현재 상태 — 무엇이 되고, 무엇을 채워야 하나

| 구성요소 | 상태 | 위치 |
|----------|------|------|
| Confluence 크롤(수집) 파이프라인 | ✅ **완성** — BFS 재귀 크롤, Bearer PAT 인증, 진행 스트리밍/취소, 연결 테스트 | `src-tauri/src/confluence.rs` |
| 설정 UI(지식 화면) | ✅ **완성** — RAG/Confluence 설정 폼 + 지식 베이스 CRUD | `src/components/KnowledgeView.tsx` |
| 워크플로우 연동(rag 단계) | ✅ **완성** — preflight가 검색 → '검색 결과' 탭 + 프롬프트 주입 | `src/components/ChatPanel.tsx` |
| **RAG 검색(`search`)** | ✅ **완성(D65)** — Samsung SDS **Fabrix rag-chat API** 실연동. 지식 화면에 연결 정보만 입력하면 동작 | `src-tauri/src/rag.rs` |
| RAG 인제스트(`ingest_page`) | ❌ **미구현 스텁** — rag-chat API에 ingest 엔드포인트가 없음(지식 자산은 Fabrix 콘솔에서 관리) | `src-tauri/src/rag.rs` |

**핵심**: RAG **검색**은 Fabrix rag-chat API(`POST /openapi/rag-chat/v1/messages`)에 질의해 **요약 답변 +
출처 문서 청크**를 받아 온다 — 지식 화면에 연결 정보(C절)만 입력하면 바로 쓸 수 있다. 요약·임베딩·벡터
검색은 Fabrix가 소유한다. **인제스트**(Confluence 원문을 RAG에 밀어넣기)는 rag-chat API로는 불가능하다
(ingest 엔드포인트 없음) — 지식 자산은 **Fabrix 콘솔**에서 만들어 채우고, 앱은 그 자산을 **조회만** 한다.

**동작(안전한 degrade — 앱은 절대 죽지 않음):**
- **검색(실행 시)**: RAG 미설정이면 rag 단계가 **에이전트 턴 없이 건너뜀**("사내 문서 검색 단계를 건너뜁니다…").
  설정 후에는 Fabrix 응답(요약 답변 + 출처)으로 정상 동작.
- **수집(Confluence → RAG ingest)**: `ingest_page`가 스텁이라 크롤은 진행하지만 각 페이지 전송이 실패 →
  "임베딩 0건 · 실패 N건". rag-chat 기반에서는 정상적인 제약(자산은 Fabrix 콘솔에서 관리).
- **연결 테스트**: 미설정/도달 불가 시 한글 안내.

### 데이터 흐름 한눈에

```
[검색 · 실행 시점]  워크플로우 rag 단계 preflight
   → rag_search(query, topK)
   → RagClient::search → POST {endpoint}/openapi/rag-chat/v1/messages
        (modelIds:[GLM 5.2], contents:[query], knowledgeAssetId, isStream:false)
   → parse_rag_response: 요약 답변 + 출처 청크 → RagHit[]
   → 캔버스 '검색 결과' 탭(HTML) + 에이전트 프롬프트에 발췌 주입

[인제스트]  rag-chat API에 ingest 엔드포인트 없음 → 미지원(자산은 Fabrix 콘솔에서 관리)
```

---

## A. RAG 검색 — 이미 구현됨(D65), 조정 포인트만 확인

RAG **검색**은 코드로 구현돼 있다(추가 개발 불필요). 지식 화면에 연결 정보(C절)만 넣으면 동작한다. 사내
Fabrix 인스턴스가 아래 **기본 계약과 다를 때만** `src-tauri/src/rag.rs`를 조정한다.

### A-1. 호출 계약 (현재 구현)

- **엔드포인트**: 검색 `POST {endpoint}/openapi/rag-chat/v1/messages`, 연결 테스트 `GET {endpoint}/openapi/rag-chat/v1/models`.
  `{endpoint}`는 지식 화면 **ENDPOINT_URL** 값(끝의 `/`는 저장 시 제거).
- **헤더**: `x-fabrix-client`(= 지식 화면 첫 키), `x-openapi-token`(= 두 번째 키, "Bearer …" 포함).
  `x-generative-ai-user-email`은 **보내지 않는다**(현재 결정 — 필요해지면 A-2의 5번대로 추가).
- **요청 body**: `{ modelIds:[GLM 5.2], contents:[query], isStream:false, llmConfig:{}, systemPrompt:"", knowledgeAssetId }`.
  모델은 GLM 5.2 고정(`GLM_5_2_MODEL_ID` 상수 `019f23a1-…`). `knowledgeAssetId`는 설정값이고, 비우면 샘플
  자산 상수(`019f5a11-…`)로 폴백한다.
- **응답 매핑**(`parse_rag_response`, 비스트림 단일 JSON): `content`(요약 답변)를 맨 앞 `RagHit`
  (title "RAG 요약 답변")으로, `contentReferences[].references[]`(없으면 top-level `references[]`)의 출처 청크를
  뒤이어 `RagHit`(title/`link`→url/`content`→snippet)으로. `top_k`는 출처 개수 상한. `status`가 FAIL/ERROR면 에러.

돌려주는 데이터(`RagHit`, `snippet`만 필수): `title?`(제목) / `url?`(출처 링크) / `snippet`(발췌 본문) /
`score?`(표시용, 현재 미사용).

### A-2. 사내 계약이 다를 때 조정할 곳 (`src-tauri/src/rag.rs`)

1. **경로**가 다르면 `search`/`probe_models`의 `format!("{}/openapi/rag-chat/v1/...", ...)`를 바꾼다.
2. **헤더 이름**이 다르면 `attach_headers`(헤더 2개를 붙이는 헬퍼)를 바꾼다.
3. **모델**을 바꾸려면 `GLM_5_2_MODEL_ID` 상수(또는 연결 테스트의 `/models`로 얻은 id)를 수정한다.
4. **응답 형태**가 다르면 `parse_rag_response`의 필드 접근(`content`/`contentReferences`/`references`/`link`/
   `title`/`filename`)을 사내 응답에 맞춘다(순수 함수라 캡처한 샘플 JSON으로 단위테스트 가능).
5. **이메일 헤더**가 필수면 `RagConfig`에 `user_email` 필드 + 폼 입력을 추가하고 `attach_headers`에
   `x-generative-ai-user-email`을 붙인다(D65 재검토 항목).

### A-3. 인제스트(`ingest_page`)는 스텁 유지

rag-chat API에는 문서 ingest 엔드포인트가 없다. 지식 자산은 **Fabrix 콘솔**에서 만들어 채운다. 따라서
Confluence 크롤 → RAG ingest 경로(B절 "수집 시작")는 rag-chat 기반에서는 임베딩되지 않는다(스텁이
"미구현" 한글 Err을 반환 → per-page 실패). 사내에 별도 ingest API가 있으면 `RagClient::ingest_page`
(rag.rs)에 그 호출을 채우면 된다(설정 로드·클라이언트·`attach_headers`는 이미 있음).

### A-4. 참고 상수·설정

- **타임아웃**: `HTTP_TIMEOUT = 120초` + `CONNECT_TIMEOUT = 30초`(`rag.rs`). RAG 검색이 워크플로우 턴
  중간에 실행되므로 느린 사내 백엔드를 위해 넉넉하다(D53). 필요하면 조정.
- **TLS**: `RagClient::new`의 `allow_invalid_certs`는 **Confluence 설정의 "TLS 인증서 검증 생략"과 같은
  값을 공유**한다(기본 false). reqwest가 native-tls=schannel로 빌드되어 **Windows 인증서 저장소**를
  신뢰하므로, 사내 TLS 재서명 프록시가 있으면 **그 CA를 Windows 저장소에 설치**하는 것이 정석이다.
  `danger_accept_invalid_certs`(TLS 예외 체크)는 최후 수단(D48).

### A-5. 빌드·테스트

Rust 빌드는 **MSVC 환경 필수**다(Git Bash·순수 PowerShell 금지 — 상세는
[06-build-and-environment.md](design/06-build-and-environment.md)). "Developer PowerShell for VS 2022"
또는 `vcvars64.bat` 초기화 후:

```powershell
# 단위 테스트 (파서·설정은 플랫폼 무관)
cargo test --manifest-path src-tauri\Cargo.toml

# 개발 실행 / 단독 exe 빌드
npm run tauri dev
npm run tauri build -- --no-bundle
```

> `rag.rs`는 `parse_rag_response` 순수 파서 단위테스트(요약 답변 prepend / `contentReferences.link` 매핑 /
> top-level `references` 폴백 / FAIL status → Err / top_k 상한 / 비-JSON → Err)와 `ingest_stub_…`(스텁이
> "미구현" 반환), `empty_endpoint_is_rejected`, `rag_hit_serde_camel_case`를 포함한다. 계약을 바꾸면 파서
> 테스트를 새 샘플로 갱신한다.

배포는 GitHub Actions `Release` 워크플로우(수동 트리거, 단독 exe — D43,
[06-build-and-environment.md](design/06-build-and-environment.md)).

---

## B. Confluence 수집 설정·실행 (지식 화면 ②)

앱 좌측 내비게이션에서 **지식**(`Library` 아이콘)으로 이동 → "Confluence 수집" 카드.

### B-1. 입력 필드

| 필드 | 설명 |
|------|------|
| **Base URL** | 컨텍스트 경로 포함(예: `https://wiki.example.com/confluence`). REST 호출의 base. |
| **Personal Access Token** | **Server/DC용 Bearer PAT**. **읽기 전용 권장**. (Atlassian **Cloud의 Basic 인증은 v1 범위 밖** — Server/DC PAT만 지원.) |
| **루트 페이지 ID** | 이 페이지의 **하위 트리를 재귀 수집**(예: `123456789`). |
| **스페이스 키** | 루트 페이지 ID를 비우면, 이 스페이스의 **모든 페이지를 평면 수집**(예: `OPS`). |
| **TLS 인증서 검증 생략** | 위험 옵션. 사내 프록시 CA를 Windows 저장소에 설치하는 것이 안전한 해법. |

- 루트 페이지 ID와 스페이스 키는 **둘 중 하나**를 쓴다(루트 우선; 둘 다 비면 시작점 없음 오류).
- 수집 상한(코드 고정, `confluence.rs` 33–36행): **최대 2000페이지 · 깊이 10 · 페이지당 100건씩 조회**.

### B-2. 절차

1. **설정 저장** — 필드 입력 후 "저장". (Base URL을 비우고 저장하면 설정 해제.)
2. **연결 테스트** — "연결 테스트" 버튼으로 `probe_confluence` 실행. 성공하면 루트(또는 스페이스 첫)
   페이지 제목이 `연결됨 — "제목"`으로 뜬다.
3. **수집 시작** — "수집 시작" 버튼. **Confluence와 RAG 설정이 둘 다 저장돼 있어야 활성화**되지만,
   **rag-chat API에는 ingest 엔드포인트가 없어 실제 임베딩은 되지 않는다**(A-3 — "임베딩 0건 · 실패 N건").
   지식 자산은 Fabrix 콘솔에서 관리한다. 사내에 별도 ingest API가 있으면 `ingest_page`를 채운다.

### B-3. 진행 관찰·중지

- 진행 패널에 **수집 / 임베딩 / 실패** 카운터와 현재 페이지 티커가 표시된다.
- **다른 화면으로 이동해도 수집은 계속되고**, 지식 화면으로 돌아오면 실시간 현황이 그대로 보인다(모듈
  싱글턴 스토어 — D51). 수집 중에는 내비게이션 '지식' 아이콘에 점(pulse)이 표시된다.
- "중지" 버튼으로 취소(요청 사이에 취소 플래그를 확인 → `end{canceled}`).
- 수집 중에는 Confluence 설정 저장이 비활성(실행 중 크롤은 시작 시점 설정을 사용).
- 페이지별 실패는 **계속 진행**(목록에 최대 5건 표시), 치명적 실패(루트 조회 실패/시작점 없음)는 중단.

---

## C. RAG 검색 설정 (지식 화면 ①)

"RAG 검색 설정" 카드. 이 5개 값만 입력·저장하면 RAG 검색이 동작한다(추가 개발 불필요 — A절 참조).

| 필드 | 설명 |
|------|------|
| **ENDPOINT_URL** | Fabrix rag-chat base URL(예: `https://nsds-api.fabrix-s.samsungsds.com/sds/trial/api-rag-chat`). 뒤에 `/openapi/rag-chat/v1/...`이 붙는다. |
| **x-fabrix-client** | 클라이언트 키(요청 헤더 값). 비우면 미사용. |
| **x-openapi-token** | 토큰(요청 헤더 값, "Bearer …" 포함). 비우면 미사용. |
| **Knowledge Asset ID** | 조회할 지식 자산 id. **비우면 샘플 자산으로 폴백**(즉시 동작). Fabrix 콘솔에서 자산을 만든 뒤 그 id로 교체. |
| **Top K** | 검색 결과(출처 청크) 개수 상한(기본 5). 요약 답변은 항상 별도 포함. |

- **연결 테스트**: `probe_rag`가 rag-chat `/models`를 조회해 `연결됨 (N개 모델)`을 표시한다(assetId 불필요).
  실패 시 HTTP/연결 오류가 (사용자 문구로) 그대로 뜬다.
- **비밀값은 평문 저장**된다(`settings.json`). 로컬 단일 사용자 앱 전제 — **읽기 전용 키 권장**(D50).
- **레거시 주의**: 구버전 설정의 `apiKey` 필드는 무시되고 헤더 키는 기본 미설정이 된다(재입력 필요).

---

## D. 지식 베이스 (지식 화면 ③) — RAG와 다른 것

혼동하기 쉬운데, "지식 베이스" 카드는 **RAG와 별개**다.

- **제목 + 본문**을 단순 저장(임베딩·벡터 검색 아님). knowledge 기반 단계가 실행 시 **프롬프트에 직접
  주입**(전체 16KB 상한)한다.
- 워크플로우 산출물을 저장한 **산출물(artifact) 항목**은 요약 + 첨부 문서 절대경로 인덱스로 주입되고,
  원문 읽기 접근이 부여된다(D59).
- 즉, **RAG = 사내 문서(Confluence) 대량 검색**, **지식 베이스 = 작업 방식/산출물 소량 직접 주입**. 둘은
  기반 3단계에서 각각 rag 단계·knowledge 단계로 쓰인다.

---

## E. 실행 시점 — 워크플로우에서 어떻게 쓰이나

카테고리 작업을 시작하면 기반 3단계(코드베이스 → **RAG 검색** → 지식) 중 **rag 단계**에서:

1. `ChatPanel.stepPreflight`가 `buildRagQuery(런처 프롬프트 + 옵션 답변, 500자 상한)`으로 질의를 만든다.
2. `rag_search(query)` 호출 → 성공 시 **관련성 판단 턴**(`judgeRagRelevance`, 격리 실행 — D70)이 검색
   결과가 이번 작업과 관련 있는지 판정한다:
   - **관련 있음**: 판단 턴이 정리한 섹션을 캔버스 **'검색 결과' 탭**(sandbox iframe, `ragCuratedHtml`)에
     보기 좋게 표시하고, `formatRagContext`로 에이전트 턴 프롬프트에 원본 발췌를 주입한다.
   - **관련 적음**: 탭·주입 없이 건너뛴다(사용자에게 무관한 정보를 보여주지 않음).
   - **판단 실패(형식 미준수 등)**: fail-open — 원본 결과를 그대로 표시·주입(`ragResultHtml`, 기존 동작).
3. **비차단 degrade**: 미설정 → 건너뜀("지식 화면에서 등록"), 0건 → 건너뜀, 에러 → 건너뜀(사용자 문구),
   preflight 중 Stop → 취소(판단 턴도 함께 취소). 어느 경우도 대화가 깨지지 않는다(D44/D70).

**카테고리별**:
- `plan`·`query`·`change`: 완전 기반 3단계(코드베이스 + RAG + 지식).
- `guide`(운영 가이드): 코드베이스를 뺀 **RAG + 지식**만(부분 foundation — D63). 사내 문서를 보기 좋게
  시각화하는 것이 이 카테고리의 강점이라, **'검색 결과' 탭**이 특히 유용하다.

상세: [07-workspace-and-runs.md](design/07-workspace-and-runs.md)·[08-guided-flows-and-skills.md](design/08-guided-flows-and-skills.md).

---

## F. 문제 해결 (Troubleshooting)

| 증상 | 확인 |
|------|------|
| **검색 단계가 계속 "건너뜀"** | (1) 지식 화면에 ENDPOINT_URL·헤더 키를 저장했는가? (2) 연결 테스트가 초록색인가? (3) Knowledge Asset ID가 유효한가(비우면 샘플)? (4) 사내망/프록시로 도달 가능한가? |
| **연결 테스트 실패(HTTP 4xx/5xx·연결 오류)** | ENDPOINT_URL·헤더 키 확인, 사내망/VPN·프록시 TLS 확인(아래 BadSignature 행). Fabrix가 이메일 헤더를 요구하면 A-2의 5번대로 필드 추가. |
| **수집이 "임베딩 0건 · 실패 N건"** | rag-chat API에는 ingest가 없어 정상적 제약이다(A-3). 지식 자산은 Fabrix 콘솔에서 관리. 사내 별도 ingest API가 있으면 `ingest_page`를 채운다. |
| **TLS `BadSignature`/인증서 오류** | 사내 프록시 CA를 **Windows 인증서 저장소에 설치**(정석) 또는 "TLS 인증서 검증 생략" 체크(위험). reqwest=schannel이라 OS 저장소 신뢰(D48·[06](design/06-build-and-environment.md)). |
| **수집 0건/루트 조회 실패** | 루트 페이지 ID 또는 스페이스 키 정확성, PAT 권한(읽기 전용이라도 대상 스페이스 접근 필요), Base URL 컨텍스트 경로. |
| **설정이 초기화된 것 같음** | `settings.json` 파손 시 기본값으로 동작하고 원본을 `settings.json.corrupt`로 보존(D56). |

비밀값 저장 위치: `%USERPROFILE%\.operation-wizard\settings.json`(RAG 키·Confluence PAT는 평문 — D72로
홈 루트로 통일; 과거 `%APPDATA%\com.shi.operationwizard\settings.json`에 있던 값은 첫 실행 시 자동 이전).

---

## G. 관련 문서

- 설계 근거(결정 로그): [05-decisions.md](design/05-decisions.md) — D46(검색 결과 탭)·D48(지식 뷰/reqwest/
  Confluence 수집/RAG 어댑터)·D50(RagConfig 헤더 3값)·D51(수집 관찰성)·D53(타임아웃)·D59(산출물 지식)·
  D63(guide 부분 foundation)·**D65(RAG 검색 Fabrix rag-chat 실연동)**.
- 실행 엔진·기반 3단계: [07-workspace-and-runs.md](design/07-workspace-and-runs.md).
- 가이드 플로우·스킬: [08-guided-flows-and-skills.md](design/08-guided-flows-and-skills.md).
- 빌드·사내망·TLS: [06-build-and-environment.md](design/06-build-and-environment.md).
- 프로젝트 전반: [PROJECT_GUIDE.md](PROJECT_GUIDE.md).
