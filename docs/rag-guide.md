# 사내 문서 RAG 검색 가이드

이 문서는 Operation Wizard의 **사내 문서 RAG 검색**을 **처음부터 실제로 동작**시키기 위한 실무 가이드다.
개념·설계 근거는 `docs/design/`(특히 [05-decisions.md](design/05-decisions.md) D46/D48/D50/D53/D59/D63/D65/
D70/D96/D98)에 있고, 이 문서는 **어디에 무엇을 입력하고, 안 될 때 무엇을 확인하는지**만 다룬다.

> 대상: 이 앱을 사내에서 운영·배포하는 담당자. **RAG 검색은 지식 화면 입력(B절)만으로 동작**하고,
> A절(코드 조정)은 사내 Fabrix 계약이 기본과 다를 때만 필요하다.

> **Confluence 수집 기능은 제거되었다(D98).** 예전 지식 화면의 "Confluence 수집(MCP)" 카드는 페이지 ID나
> 검색어를 미리 알아야 해서 실사용이 어려웠다. Confluence 등 사내 문서는 **RAG 지식 자산**(Fabrix 콘솔에서
> 관리)에서 검색되고, 검색 결과의 출처 링크로 원문을 연다.

---

## 0. 현재 상태

| 구성요소 | 상태 | 위치 |
|----------|------|------|
| **RAG 검색(`search`)** | ✅ **완성(D65)** — Samsung SDS **Fabrix rag-chat API** 실연동. 지식 화면에 연결 정보만 입력하면 동작 | `src-tauri/src/rag.rs` |
| 설정 UI(지식 화면) | ✅ **완성** — RAG 설정 폼 + 연결 테스트 + 지식 베이스 CRUD | `src/components/KnowledgeView.tsx` |
| 워크플로우 연동(rag 단계) | ✅ **완성** — preflight가 검색 → 관련성 판단 → '검색 결과' 탭 + 프롬프트 주입 | `src/components/ChatPanel.tsx` |
| 문서 수집(인제스트) | — rag-chat API에 ingest 엔드포인트가 없다. 지식 자산은 **Fabrix 콘솔**에서 만들어 채운다 | (앱 범위 밖) |

**핵심**: RAG **검색**은 Fabrix rag-chat API(`POST /openapi/rag-chat/v1/messages`)에 질의해 **요약 답변 +
출처 문서 청크**를 받아 온다. 요약·임베딩·벡터 검색은 Fabrix가 소유하고, 앱은 지식 자산을 **조회만** 한다.

**동작(안전한 degrade — 앱은 절대 죽지 않음):**
- RAG 미설정이면 rag 단계가 **에이전트 턴 없이 건너뜀**("RAG가 설정되지 않아 … 건너뜁니다").
- 설정 후에는 Fabrix 응답(요약 답변 + 출처)으로 정상 동작. 연결 테스트는 미설정/도달 불가 시 한글 안내.

### 데이터 흐름 한눈에

```
워크플로우 rag 단계 preflight
   → rag_search(query, topK)
   → RagClient::search → POST {endpoint}/openapi/rag-chat/v1/messages
        (modelIds:[GLM 5.2], contents:[query], knowledgeAssetId, isStream:false)
   → parse_rag_response: 요약 답변 + 출처 청크 → RagHit[]
   → 관련성 판단 턴(D70) → 캔버스 '검색 결과' 탭(HTML) + 에이전트 프롬프트에 발췌 주입
```

---

## A. RAG 검색 — 이미 구현됨(D65), 조정 포인트만 확인

RAG 검색은 코드로 구현돼 있다(추가 개발 불필요). 사내 Fabrix 인스턴스가 아래 **기본 계약과 다를 때만**
`src-tauri/src/rag.rs`를 조정한다.

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

1. **경로**가 다르면 `search`/`fetch_models`의 `format!("{}/openapi/rag-chat/v1/...", ...)`를 바꾼다.
2. **헤더 이름**이 다르면 `attach_headers`(헤더 2개를 붙이는 헬퍼)를 바꾼다.
3. **모델**을 바꾸려면 `GLM_5_2_MODEL_ID` 상수(또는 연결 테스트의 `/models`로 얻은 id)를 수정한다.
4. **응답 형태**가 다르면 `parse_rag_response`의 필드 접근(`content`/`contentReferences`/`references`/`link`/
   `title`/`filename`)을 사내 응답에 맞춘다(순수 함수라 캡처한 샘플 JSON으로 단위테스트 가능).
5. **이메일 헤더**가 필수면 `RagConfig`에 `user_email` 필드 + 폼 입력을 추가하고 `attach_headers`에
   `x-generative-ai-user-email`을 붙인다(D65 재검토 항목).

### A-3. 참고 상수·설정

- **타임아웃**: `HTTP_TIMEOUT = 120초` + `CONNECT_TIMEOUT = 30초`(`rag.rs`). RAG 검색이 워크플로우 턴
  중간에 실행되므로 느린 사내 백엔드를 위해 넉넉하다(D53). 필요하면 조정.
- **프록시**: 사내 직접 도달 엔드포인트라 환경 프록시를 무시하고 직접 연결한다(`.no_proxy()` — D66).
- **TLS**: 인증서 검증은 항상 켜져 있다. reqwest가 native-tls=schannel로 빌드되어 **Windows 인증서
  저장소**를 신뢰하므로, 사내 TLS 재서명 프록시가 있으면 **그 CA를 Windows 저장소에 설치**한다(D48).

### A-4. 빌드·테스트

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
> top-level `references` 폴백 / FAIL status → Err / top_k 상한 / 비-JSON → Err)와 `empty_endpoint_is_rejected`,
> `rag_hit_serde_camel_case`를 포함한다. 계약을 바꾸면 파서 테스트를 새 샘플로 갱신한다.

배포는 GitHub Actions `Release` 워크플로우(수동 트리거, 단독 exe — D43,
[06-build-and-environment.md](design/06-build-and-environment.md)).

---

## B. RAG 검색 설정 (지식 화면)

앱 좌측 내비게이션에서 **지식**(`Library` 아이콘)으로 이동 → "RAG 검색 설정" 카드. 이 5개 값만 입력·저장하면
RAG 검색이 동작한다(추가 개발 불필요 — A절 참조).

| 필드 | 설명 |
|------|------|
| **ENDPOINT_URL** | Fabrix rag-chat base URL(예: `https://nsds-api.fabrix-s.samsungsds.com/sds/trial/api-rag-chat`). 뒤에 `/openapi/rag-chat/v1/...`이 붙는다. |
| **x-fabrix-client** | 클라이언트 키(요청 헤더 값). 비우면 미사용. |
| **x-openapi-token** | 토큰(요청 헤더 값, "Bearer …" 포함). 비우면 미사용. |
| **Knowledge Asset ID** | 조회할 지식 자산 id. **비우면 샘플 자산으로 폴백**(즉시 동작). Fabrix 콘솔에서 자산을 만든 뒤 그 id로 교체. |
| **Top K** | 검색 결과(출처 청크) 개수 상한(기본 5). 요약 답변은 항상 별도 포함. |

- **연결 테스트**: `probe_rag`가 rag-chat `/models`를 조회해 `연결됨 (N개 모델)`을 표시한다(assetId 불필요).
  실패 시 HTTP/연결 오류가 (사용자 문구로) 그대로 뜬다. 조회한 모델 목록은 캐시되어 다음부터 네트워크 없이 표시된다(D66).
- **비밀값은 평문 저장**된다(`settings.json`). 로컬 단일 사용자 앱 전제 — **읽기 전용 키 권장**(D50).
- **레거시 주의**: 구버전 설정의 `apiKey` 필드는 무시되고 헤더 키는 기본 미설정이 된다(재입력 필요).
  구버전의 `confluence` 설정 섹션도 무시되며 다음 저장에서 사라진다(D98).

---

## C. 지식 베이스 (지식 화면) — RAG와 다른 것

혼동하기 쉬운데, "지식 베이스"는 **RAG와 별개**다.

- **제목 + 본문**을 단순 저장(임베딩·벡터 검색 아님). knowledge 기반 단계가 실행 시 **프롬프트에 직접
  주입**(전체 16KB 상한)한다.
- 워크플로우 산출물을 저장한 **산출물(artifact) 항목**은 요약 + 첨부 문서 절대경로 인덱스로 주입되고,
  원문 읽기 접근이 부여된다(D59).
- 즉, **RAG = 사내 문서(Confluence 등) 대량 검색**, **지식 베이스 = 작업 방식/산출물 소량 직접 주입**. 둘은
  기반 3단계에서 각각 rag 단계·knowledge 단계로 쓰인다.
- 예전 Confluence 수집으로 만든 항목(제목 "Confluence 수집 — …")이 남아 있으면 일반 산출물 항목처럼 주입된다.
  필요 없으면 지식 화면에서 삭제한다(D98).

---

## D. 실행 시점 — 워크플로우에서 어떻게 쓰이나

카테고리 작업을 시작하면 기반 3단계(코드베이스 → **RAG 검색** → 지식) 중 **rag 단계**에서:

1. 요구사항 폼 하단의 **'사내 문서 RAG 검색' 스위치**가 꺼져 있으면 검색 없이 건너뛴다(D96).
2. `ChatPanel.stepPreflight`가 `buildRagQuery(요구사항 + 옵션 답변, 500자 상한)`으로 질의를 만든다.
3. `rag_search(query)` 호출 → 성공 시 **관련성 판단 턴**(`judgeRagRelevance`, 격리 실행 — D70)이 검색
   결과가 이번 작업과 관련 있는지 판정한다:
   - **관련 있음**: 판단 턴이 정리한 섹션을 캔버스 **'검색 결과' 탭**(sandbox iframe, `ragCuratedHtml`)에
     보기 좋게 표시하고, `formatRagContext`로 에이전트 턴 프롬프트에 원본 발췌를 주입한다. 탭의 출처 링크는
     OS 기본 브라우저로 열린다(D76).
   - **관련 적음**: 탭·주입 없이 건너뛴다(사용자에게 무관한 정보를 보여주지 않음).
   - **판단 실패(형식 미준수 등)**: fail-open — 원본 결과를 그대로 표시·주입(`ragResultHtml`).
4. **비차단 degrade**: 미설정 → 건너뜀("지식 화면에서 등록"), 0건 → 건너뜀, 에러 → 건너뜀(사용자 문구),
   preflight 중 Stop → 취소(판단 턴도 함께 취소). 어느 경우도 대화가 깨지지 않는다(D44/D70).

**카테고리별**:
- `plan`·`query`·`change`: 완전 기반 3단계(코드베이스 + RAG + 지식).
- `guide`(운영 가이드): 코드베이스를 뺀 **RAG + 지식**만(부분 foundation — D63). 사내 문서를 보기 좋게
  시각화하는 것이 이 카테고리의 강점이라, **'검색 결과' 탭**이 특히 유용하다.

상세: [07-workspace-and-runs.md](design/07-workspace-and-runs.md)·[08-guided-flows-and-skills.md](design/08-guided-flows-and-skills.md).

---

## E. 문제 해결 (Troubleshooting)

| 증상 | 확인 |
|------|------|
| **검색 단계가 계속 "건너뜀"** | (1) 지식 화면에 ENDPOINT_URL·헤더 키를 저장했는가? (2) 연결 테스트가 초록색인가? (3) Knowledge Asset ID가 유효한가(비우면 샘플)? (4) 요구사항 폼의 RAG 스위치가 켜져 있는가? (5) 사내망으로 도달 가능한가? |
| **연결 테스트 실패(HTTP 4xx/5xx·연결 오류)** | ENDPOINT_URL·헤더 키 확인, 사내망/VPN·TLS 확인(아래 인증서 행). Fabrix가 이메일 헤더를 요구하면 A-2의 5번대로 필드 추가. |
| **"관련이 적어 표시하지 않습니다"가 자주 뜸** | 관련성 판단 턴이 무관하다고 본 경우다(D70). 요구사항을 더 구체적으로 쓰거나, 지식 자산(Knowledge Asset ID)이 해당 업무 문서를 담고 있는지 확인. |
| **TLS `BadSignature`/인증서 오류** | 사내 프록시 CA를 **Windows 인증서 저장소에 설치**한다. reqwest=schannel이라 OS 저장소를 신뢰한다(D48·[06](design/06-build-and-environment.md)). |
| **설정이 초기화된 것 같음** | `settings.json` 파손 시 기본값으로 동작하고 원본을 `settings.json.corrupt`로 보존(D56). |

비밀값 저장 위치: `%USERPROFILE%\.operation-wizard\settings.json`(RAG 키는 평문 — D72로 홈 루트로 통일;
과거 `%APPDATA%\com.shi.operationwizard\settings.json`에 있던 값은 첫 실행 시 자동 이전).

---

## F. 관련 문서

- 설계 근거(결정 로그): [05-decisions.md](design/05-decisions.md) — D46(검색 결과 탭)·D48(지식 뷰/reqwest)·
  D50(RagConfig 헤더 값)·D53(타임아웃)·D59(산출물 지식)·D63(guide 부분 foundation)·**D65(RAG 검색 Fabrix
  rag-chat 실연동)**·D70(관련성 판단)·D96(RAG 스위치)·D98(Confluence 수집 제거).
- 실행 엔진·기반 3단계: [07-workspace-and-runs.md](design/07-workspace-and-runs.md).
- 가이드 플로우·스킬: [08-guided-flows-and-skills.md](design/08-guided-flows-and-skills.md).
- 빌드·사내망·TLS: [06-build-and-environment.md](design/06-build-and-environment.md).
- 프로젝트 전반: [PROJECT_GUIDE.md](PROJECT_GUIDE.md).
