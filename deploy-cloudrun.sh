#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   PROJECT_ID="my-project" REGION="us-central1" SERVICE_NAME="qr-permisos-backend" ./backend/deploy-cloudrun.sh
#
# Optional env vars:
#   BUCKET_NAME (default: qr-permisos-pdfs-${PROJECT_ID})
#   RUNTIME_SA_NAME (default: cloud-run-pdf-sa)
#
# Run this script from the repository root (folder that contains backend/ and frontend/).

: "${PROJECT_ID:?PROJECT_ID is required}"
REGION="${REGION:-us-central1}"
SERVICE_NAME="${SERVICE_NAME:-qr-permisos-backend}"
BUCKET_NAME="${BUCKET_NAME:-qr-permisos-pdfs-${PROJECT_ID}}"
RUNTIME_SA_NAME="${RUNTIME_SA_NAME:-cloud-run-pdf-sa}"
RUNTIME_SA="${RUNTIME_SA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"

echo "==> Using:"
echo "  PROJECT_ID=${PROJECT_ID}"
echo "  REGION=${REGION}"
echo "  SERVICE_NAME=${SERVICE_NAME}"
echo "  BUCKET_NAME=${BUCKET_NAME}"
echo "  RUNTIME_SA=${RUNTIME_SA}"

gcloud config set project "${PROJECT_ID}"

gcloud services enable \
  run.googleapis.com \
  cloudbuild.googleapis.com \
  artifactregistry.googleapis.com \
  storage.googleapis.com

# Create bucket if it does not exist.
if ! gcloud storage buckets describe "gs://${BUCKET_NAME}" ~>/dev/null 2>&1; then
  gcloud storage buckets create "gs://${BUCKET_NAME}" \
    --location="${REGION}" \
    --uniform-bucket-level-access
else
  echo "Bucket gs://${BUCKET_NAME} already exists"
fi

# Create service account if it does not exist.
if ! gcloud iam service-accounts describe "${RUNTIME_SA}" >/dev/null 2>&1; then
  gcloud iam service-accounts create "${RUNTIME_SA_NAME}" \
    --display-name="Cloud Run PDF Backend SA"
else
  echo "Service account ${RUNTIME_SA} already exists"
fi

# Minimal bucket permissions for current backend behavior.
gcloud storage buckets add-iam-policy-binding "gs://${BUCKET_NAME}" \
  --member="serviceAccount:${RUNTIME_SA}" \
  --role="roles/storage.objectCreator"

gcloud storage buckets add-iam-policy-binding "gs://${BUCKET_NAME}" \
  --member="serviceAccount:${RUNTIME_SA}" \
  --role="roles/storage.objectViewer"

# Deploy Cloud Run service from source.
gcloud run deploy "${SERVICE_NAME}" \
  --source ./backend \
  --region "${REGION}" \
  --allow-unauthenticated \
  --service-account "${RUNTIME_SA}" \
  --set-env-vars "GCS_BUCKET_NAME=${BUCKET_NAME},PDFIUM_LIBRARY_PATH=/app/libpdfium.so,BIND_HOST=0.0.0.0"

BACKEND_URL="$(gcloud run services describe "${SERVICE_NAME}" --region "${REGION}" --format='value(status.url)')"

echo
echo "==> Backend URL: ${BACKEND_URL}"
echo "==> Health check:"
curl -sS "${BACKEND_URL}/health" || true

echo
echo "Set these env vars in Vercel (Project Settings -> Environment Variables):"
echo "  PDF_BACKEND_URL=${BACKEND_URL}"
echo "  NEXT_PUBLIC_PDF_BACKEND_URL=${BACKEND_URL}"
