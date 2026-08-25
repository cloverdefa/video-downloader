## Summary

<!--
請簡述這個 PR 的目的與背景。

請說明：
- 為什麼需要這個修改？
- 解決了什麼問題？
- 希望達成什麼結果？

請避免只寫「更新設定」、「修正問題」等過於籠統的描述。
-->

## Changes

<!--
請列出這個 PR 實際修改的內容。

建議使用條列式，並盡量具體說明：
- 修改了哪些 Python 程式碼、設定或 GitHub Actions？
- 新增、修改或移除了哪些功能？
- 是否影響 GUI、下載流程或使用者操作？
- 是否調整 yt-dlp、FFmpeg、Deno 或其他第三方元件的處理方式？
- 是否影響 PyInstaller Build 或 Portable Release？
-->

-

## Testing

<!--
請說明你如何驗證這次修改。

請盡可能提供：
- 測試環境
- Windows 版本
- Python 版本（如果從原始碼執行）
- yt-dlp / FFmpeg / Deno 版本（如適用）
- 測試方法
- 測試結果

如果涉及影片下載功能，建議確認：
- URL 可以正常解析
- 影片名稱可以正常顯示
- 下載進度可以正常更新
- 影片可以正常下載
- 影片與音訊可以正常合併（如適用）
- 取消下載可以正常運作
- 下載完成後可以再次下載

如果涉及 Build / Release，請確認：
- PyInstaller Build 是否成功
- 產生的 executable 是否可以正常啟動
- Portable ZIP 是否可以正常執行
- 第三方執行檔是否可以正常找到
- GitHub Actions / CI 是否成功

如果無法進行完整測試，請明確說明原因。
-->

## Breaking Changes

<!--
請確認這個 PR 是否會改變現有使用者的操作方式或行為。

如果有 Breaking Change，請明確說明：
- 哪些功能受到影響？
- 使用者需要做什麼調整？
- 是否需要重新下載 Release？
- 是否需要重新建立 Portable 環境？
- 是否需要修改設定或使用方式？

如果沒有 Breaking Change，請填寫「無」。
-->

## Related Issues

<!--
如果這個 PR 與 Issue、Discussion 或其他 PR 有關，請在此列出。

例如：

- Relates to #123
- Fixes #123
- Closes #123

如果沒有相關項目，請填寫「無」。
-->

## Additional Notes

<!--
請提供 Reviewer 需要知道的其他資訊。

例如：
- 已知限制
- 尚未解決的問題
- 需要特別注意的程式碼
- 需要 Reviewer 特別測試的功能
- 為什麼採用目前的實作方式
- 是否受到 yt-dlp、FFmpeg、Deno 或來源網站行為影響
- 是否涉及第三方元件版本更新
- 其他可能影響使用者或 Release 的資訊
-->

<details>
<summary>Checklist</summary>

- [ ] 我已確認這個 PR 的目的與修改內容已清楚說明
- [ ] 我已確認程式碼與設定符合專案規範
- [ ] 我已自行 Review 此次變更
- [ ] 我已完成適當的本機測試
- [ ] 我已確認 GitHub Actions / CI 通過（如適用）
- [ ] 我已確認 GUI 可以正常啟動（如適用）
- [ ] 我已確認影片下載功能正常（如適用）
- [ ] 我已確認下載進度與狀態顯示正常（如適用）
- [ ] 我已確認取消下載功能正常（如適用）
- [ ] 我已確認影片與音訊合併功能正常（如適用）
- [ ] 我已確認 yt-dlp / FFmpeg / Deno 相關功能正常（如適用）
- [ ] 我已確認 PyInstaller Build 正常（如適用）
- [ ] 我已確認 Portable Release 可以正常執行（如適用）
- [ ] 我已確認沒有引入 Breaking Change，或已在上方明確說明
- [ ] 我已確認相關測試已新增或更新（如適用）
- [ ] 我已確認必要的文件已新增或更新（如適用）
- [ ] 我已確認沒有提交不必要的檔案、Debug code 或暫存內容
- [ ] 我已確認變更範圍與 PR 目的相符，沒有混入無關修改
- [ ] 我已確認 Commit Message 符合 Conventional Commits 規範
- [ ] 我已確認沒有洩漏密碼、Token、Cookie、私密金鑰或其他敏感資訊
- [ ] 我已確認沒有加入不必要的第三方執行檔或大型 Binary
- [ ] 我已確認 Reviewer 可以根據 PR 說明理解這次變更

</details>
