# Dev: Vite nasłuchuje na 0.0.0.0 — w Compose proxy do API przez nazwę serwisu (API_UPSTREAM).
FROM node:20-alpine
WORKDIR /app/web

COPY web/package.json ./
RUN npm install

COPY web/ ./

EXPOSE 3000
ENV VITE_DOCKER=1
CMD ["npm", "run", "dev", "--", "--host", "0.0.0.0", "--port", "3000"]
