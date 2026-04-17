# Render + Supabase Storage Setup

## 1) Create Supabase Storage bucket
In Supabase Dashboard:
1. Go to Storage -> New bucket.
2. Bucket name: `pdf-files` (or another name, but match `SUPABASE_STORAGE_BUCKET`).
3. Keep bucket private.

## 2) Render service configuration
Create a Render Web Service for folder `backend/` using Docker.

Set environment variables from `render.env.example`:
- `SUPABASE_URL`
- `SUPABASE_SERVICE_ROLE_KEY`
- `SUPABASE_STORAGE_BUCKET` (example: `pdf-files`)
- `PDFIUM_LIBRARY_PATH=/app/libpdfium.so`
- `BIND_HOST=0.0.0.0`

Optional:
- `FILES_ROOT=/var/data/files` (only used if Supabase config is missing)

Notes:
- `PORT` is provided automatically by Render; backend already supports it.
- `SUPABASE_SERVICE_ROLE_KEY` must be secret and only on backend.

## 3) Vercel frontend configuration
In Vercel Project Settings -> Environment Variables:
- `PDF_BACKEND_URL=https://YOUR_RENDER_BACKEND_URL`
- `NEXT_PUBLIC_PDF_BACKEND_URL=https://YOUR_RENDER_BACKEND_URL`
- `SUPABASE_URL=https://YOUR_PROJECT_REF.supabase.co`
- `SUPABASE_SERVICE_ROLE_KEY=YOUR_SUPABASE_SERVICE_ROLE_KEY`

Then redeploy frontend.

## 4) Verify end-to-end
1. Open `https://YOUR_RENDER_BACKEND_URL/health`.
2. Generate PDFs from CSV.
3. Search by plate.
4. Edit one result and regenerate.
5. Confirm regenerated PDF is still editable from search (uses stored `_pdfFileId`).

## 5) Troubleshooting
- 401/403 to Supabase Storage: check `SUPABASE_SERVICE_ROLE_KEY`.
- 404 on download by `fileId`: object not present in storage bucket.
- Backend works locally but not on Render: verify `PDFIUM_LIBRARY_PATH=/app/libpdfium.so`.
