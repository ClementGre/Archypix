# Use a multi-stage build to create a small, optimized production image.

# 1. Build stage: Install dependencies and build the frontend application.
FROM node:24-alpine AS build
WORKDIR /app

# Copy package.json and pnpm-lock.yaml and install dependencies
COPY ./front/package.json ./front/pnpm-lock.yaml ./
RUN npm install -g pnpm && pnpm install

# Copy the rest of the application source code
COPY ./front .

# Build the application
RUN pnpm build

# 2. Production stage: Serve the built application with a lightweight web server.
FROM nginx:1.25-alpine AS prod
WORKDIR /usr/share/nginx/html

# Copy the built application from the build stage
COPY --from=build /app/dist .

# Expose port 80
EXPOSE 80

# Start Nginx
CMD ["nginx", "-g", "daemon off;"]
