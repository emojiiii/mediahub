// This file is generated from openapi/openapi.json. Do not edit it manually.
/* eslint-disable */

export interface paths {
    "/api/v1/access-keys/{access_key_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** Revoke an access key */
        delete: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    access_key_id: components["parameters"]["AccessKeyId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
            };
        };
        options?: never;
        head?: never;
        /** Update an access key */
        patch: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    access_key_id: components["parameters"]["AccessKeyId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["UpdateAccessKey"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AccessKey"];
                    };
                };
            };
        };
        trace?: never;
    };
    "/api/v1/admin/applications": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List applications for system administration */
        get: {
            parameters: {
                query?: {
                    limit?: components["parameters"]["AdminLimit"];
                };
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminApplication"][];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/applications/{application_id}/quota": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /** Change an application storage quota */
        patch: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    application_id: components["parameters"]["ApplicationId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["AdminUpdateApplicationQuota"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminApplication"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                503: components["responses"]["Unavailable"];
            };
        };
        trace?: never;
    };
    "/api/v1/admin/audit": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List deployment audit events */
        get: {
            parameters: {
                query?: {
                    limit?: components["parameters"]["AdminLimit"];
                };
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminAudit"][];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/jobs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List jobs for system administration */
        get: {
            parameters: {
                query?: {
                    limit?: components["parameters"]["AdminLimit"];
                };
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminJob"][];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/settings": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read deployment download settings */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminSettings"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /** Update deployment download settings */
        patch: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["AdminUpdateSettings"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminSettings"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        trace?: never;
    };
    "/api/v1/admin/storage": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read global storage totals */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminStorage"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/users": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List users for system administration */
        get: {
            parameters: {
                query?: {
                    limit?: components["parameters"]["AdminLimit"];
                };
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminUser"][];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/users/{user_id}/status": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        /** Suspend or reactivate a user */
        patch: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    user_id: components["parameters"]["UserId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["AdminUpdateUserStatus"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AdminUser"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                503: components["responses"]["Unavailable"];
            };
        };
        trace?: never;
    };
    "/api/v1/applications": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List applications */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Application"][];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
            };
        };
        put?: never;
        /** Create an application */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["CreateApplication"];
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Application"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/applications/{app_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read an application */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Application"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        put?: never;
        post?: never;
        /** Delete an application */
        delete: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
            };
        };
        options?: never;
        head?: never;
        /** Update an application */
        patch: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["UpdateApplication"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Application"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        trace?: never;
    };
    "/api/v1/applications/{app_id}/access-keys": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List access keys */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AccessKey"][];
                    };
                };
            };
        };
        put?: never;
        /** Create an access key */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["CreateAccessKey"];
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["CreateAccessKeyResponse"];
                    };
                };
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/audit-logs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List application audit events */
        get: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AuditEvent"][];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/forgot-password": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Request a password reset */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["ForgotPassword"];
                };
            };
            responses: {
                /** @description Request accepted */
                202: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["ForgotPasswordResponse"];
                    };
                };
                429: components["responses"]["RateLimited"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/login": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Create a session */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["Credentials"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Me"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                429: components["responses"]["RateLimited"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/logout": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Revoke the current session */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/me": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read the current identity */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Me"];
                    };
                };
                401: components["responses"]["Unauthorized"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/register": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Register a user */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["Credentials"];
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["RegistrationResponse"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                /** @description Permission denied */
                403: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Error"];
                    };
                };
                409: components["responses"]["Conflict"];
                429: components["responses"]["RateLimited"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/resend-verification": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Resend email verification */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["ForgotPassword"];
                };
            };
            responses: {
                /** @description Request accepted */
                202: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["ResendVerificationResponse"];
                    };
                };
                429: components["responses"]["RateLimited"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/reset-password": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Reset a password */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["ResetPassword"];
                };
            };
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                400: components["responses"]["InvalidRequest"];
                429: components["responses"]["RateLimited"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/sessions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List active sessions */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Session"][];
                    };
                };
                401: components["responses"]["Unauthorized"];
            };
        };
        put?: never;
        post?: never;
        /** Revoke all sessions */
        delete: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
            };
        };
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/sessions/{session_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** Revoke one session */
        delete: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    session_id: components["parameters"]["SessionId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/verify-email": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Verify an email address */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["OneTimeToken"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AuthStatus"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                429: components["responses"]["RateLimited"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/buckets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List buckets */
        get: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Bucket"][];
                    };
                };
                401: components["responses"]["Unauthorized"];
            };
        };
        put?: never;
        /** Create a bucket */
        post: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                    "Idempotency-Key"?: components["parameters"]["IdempotencyKey"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["CreateBucket"];
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Bucket"];
                    };
                };
                /** @description Request accepted */
                202: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                409: components["responses"]["Conflict"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/buckets/{name}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read a bucket */
        get: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    name: components["parameters"]["BucketName"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Bucket"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        put?: never;
        post?: never;
        /** Delete a bucket */
        delete: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    name: components["parameters"]["BucketName"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
            };
        };
        options?: never;
        head?: never;
        /** Update a bucket */
        patch: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    name: components["parameters"]["BucketName"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["UpdateBucket"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Bucket"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        trace?: never;
    };
    "/api/v1/capabilities": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read deployment capabilities */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Capabilities"];
                    };
                };
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/jobs/{job_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read an asynchronous job */
        get: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    job_id: components["parameters"]["JobId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AsyncJobDetails"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        put?: never;
        post?: never;
        /** Cancel an asynchronous job */
        delete: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    job_id: components["parameters"]["JobId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AsyncJob"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
            };
        };
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/me": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read the current identity */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Me"];
                    };
                };
                401: components["responses"]["Unauthorized"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/media": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List media or Bucket-scoped virtual directories with a stable cursor */
        get: {
            parameters: {
                query?: {
                    bucket?: components["parameters"]["MediaBucket"];
                    status?: components["parameters"]["MediaStatus"];
                    mime?: components["parameters"]["MediaMime"];
                    created_from?: components["parameters"]["CreatedFrom"];
                    created_before?: components["parameters"]["CreatedBefore"];
                    prefix?: components["parameters"]["ObjectPrefix"];
                    delimiter?: components["parameters"]["Delimiter"];
                    limit?: components["parameters"]["Limit"];
                    cursor?: components["parameters"]["Cursor"];
                };
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["MediaPage"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        put?: never;
        /** Upload media with multipart form data */
        post: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "multipart/form-data": components["schemas"]["UploadMedia"];
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Media"];
                    };
                };
                409: components["responses"]["Conflict"];
                413: components["responses"]["PayloadTooLarge"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/media/batch": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Run or schedule a media batch operation */
        post: {
            parameters: {
                query?: never;
                header: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                    "Idempotency-Key": components["parameters"]["BatchIdempotencyKey"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["BatchMediaRequest"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["BatchMediaResponse"];
                    };
                };
                /** @description Request accepted */
                202: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["AsyncJobReceipt"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                503: components["responses"]["Unavailable"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/uploads": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Create an upload session */
        post: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["CreateUploadSession"];
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["CreateUploadSessionResponse"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                413: components["responses"]["PayloadTooLarge"];
                415: components["responses"]["UnsupportedMediaType"];
                503: components["responses"]["Unavailable"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/uploads/{upload_session_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read an upload session */
        get: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    upload_session_id: components["parameters"]["UploadSessionId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["UploadSession"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        /** Cancel an upload session */
        delete: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    upload_session_id: components["parameters"]["UploadSessionId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                503: components["responses"]["Unavailable"];
            };
        };
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/uploads/{upload_session_id}/complete": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Complete an upload session */
        post: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    upload_session_id: components["parameters"]["UploadSessionId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["CompleteUploadSession"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["CompleteUploadSessionResponse"];
                    };
                };
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["CompleteUploadSessionResponse"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                422: components["responses"]["UnprocessableContent"];
                503: components["responses"]["Unavailable"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/uploads/{upload_session_id}/content": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /** Upload content using a short-lived capability */
        put: {
            parameters: {
                query?: never;
                header: {
                    "Content-Length": components["parameters"]["ContentLength"];
                    "Content-Type": components["parameters"]["ContentType"];
                };
                path: {
                    upload_session_id: components["parameters"]["UploadSessionId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/octet-stream": string;
                };
            };
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                400: components["responses"]["InvalidRequest"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                413: components["responses"]["PayloadTooLarge"];
                415: components["responses"]["UnsupportedMediaType"];
                422: components["responses"]["UnprocessableContent"];
                503: components["responses"]["Unavailable"];
            };
        };
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/webhooks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List webhook endpoints */
        get: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Webhook"][];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
            };
        };
        put?: never;
        /** Create a webhook endpoint */
        post: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path?: never;
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["CreateWebhook"];
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["CreateWebhookResponse"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/webhooks/{webhook_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** Delete a webhook endpoint */
        delete: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    webhook_id: components["parameters"]["WebhookId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        options?: never;
        head?: never;
        /** Update a webhook endpoint */
        patch: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    webhook_id: components["parameters"]["WebhookId"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["UpdateWebhook"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["UpdateWebhookResponse"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        trace?: never;
    };
    "/api/v1/webhooks/{webhook_id}/deliveries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List webhook delivery history */
        get: {
            parameters: {
                query?: {
                    status?: components["parameters"]["DeliveryStatus"];
                    limit?: components["parameters"]["Limit"];
                    cursor?: components["parameters"]["Cursor"];
                };
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    webhook_id: components["parameters"]["WebhookId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["WebhookDeliveryPage"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/webhooks/{webhook_id}/deliveries/{event_id}/replay": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Replay a terminal webhook delivery */
        post: {
            parameters: {
                query?: never;
                header?: {
                    "X-MediaHub-App-Id"?: components["parameters"]["ApplicationContext"];
                };
                path: {
                    webhook_id: components["parameters"]["WebhookId"];
                    event_id: components["parameters"]["EventId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Request accepted */
                202: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
            };
        };
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/health/live": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Process liveness */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/health/ready": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Dependency readiness */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/metrics": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read Prometheus deployment metrics */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "text/plain": string;
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                503: components["responses"]["Unavailable"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/{app_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List Buckets by Application path */
        get: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Bucket"][];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/{app_id}/{bucket}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** List objects or virtual directories by Bucket path */
        get: {
            parameters: {
                query?: {
                    prefix?: components["parameters"]["ObjectPrefix"];
                    delimiter?: components["parameters"]["Delimiter"];
                    limit?: components["parameters"]["Limit"];
                    cursor?: components["parameters"]["Cursor"];
                };
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["MediaPage"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        /** Create a private Bucket by path */
        put: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Bucket"];
                    };
                };
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Bucket"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                409: components["responses"]["Conflict"];
            };
        };
        post?: never;
        /** Delete an empty Bucket by path */
        delete: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response with no body */
                204: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
            };
        };
        options?: never;
        /** Check a Bucket by path */
        head: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        patch?: never;
        trace?: never;
    };
    "/{app_id}/{bucket}/{object_key}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read object content by Application, Bucket, and Object Key */
        get: {
            parameters: {
                query?: {
                    w?: components["parameters"]["VariantWidth"];
                    h?: components["parameters"]["VariantHeight"];
                    fit?: components["parameters"]["VariantFit"];
                    quality?: components["parameters"]["VariantQuality"];
                    format?: components["parameters"]["VariantFormat"];
                    blur?: components["parameters"]["VariantBlur"];
                    crop?: components["parameters"]["VariantCrop"];
                    background?: components["parameters"]["VariantBackground"];
                };
                header?: {
                    Range?: components["parameters"]["Range"];
                    "If-None-Match"?: components["parameters"]["IfNoneMatch"];
                };
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                    object_key: components["parameters"]["ObjectKey"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/octet-stream": string;
                    };
                };
                /** @description Partial content */
                206: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/octet-stream": string;
                    };
                };
                /** @description Not modified */
                304: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                400: components["responses"]["InvalidRequest"];
                404: components["responses"]["NotFound"];
                413: components["responses"]["PayloadTooLarge"];
                415: components["responses"]["UnsupportedMediaType"];
                /** @description Range not satisfiable */
                416: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                422: components["responses"]["UnprocessableContent"];
            };
        };
        /** Create immutable object content by path */
        put: {
            parameters: {
                query?: never;
                header: {
                    "Content-Length": components["parameters"]["ContentLength"];
                    "Content-Type": components["parameters"]["ContentType"];
                };
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                    object_key: components["parameters"]["ObjectKey"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/octet-stream": string;
                };
            };
            responses: {
                /** @description Resource created */
                201: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
                413: components["responses"]["PayloadTooLarge"];
                415: components["responses"]["UnsupportedMediaType"];
                503: components["responses"]["Unavailable"];
            };
        };
        /** Create a signed object URL by path */
        post: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                    object_key: components["parameters"]["ObjectKey"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["SignedMediaUrl"];
                    };
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
            };
        };
        /** Schedule object deletion by path */
        delete: {
            parameters: {
                query?: never;
                header?: never;
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                    object_key: components["parameters"]["ObjectKey"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Request accepted */
                202: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
            };
        };
        options?: never;
        /** Read object headers by Application, Bucket, and Object Key */
        head: {
            parameters: {
                query?: {
                    w?: components["parameters"]["VariantWidth"];
                    h?: components["parameters"]["VariantHeight"];
                    fit?: components["parameters"]["VariantFit"];
                    quality?: components["parameters"]["VariantQuality"];
                    format?: components["parameters"]["VariantFormat"];
                    blur?: components["parameters"]["VariantBlur"];
                    crop?: components["parameters"]["VariantCrop"];
                    background?: components["parameters"]["VariantBackground"];
                };
                header?: {
                    Range?: components["parameters"]["Range"];
                    "If-None-Match"?: components["parameters"]["IfNoneMatch"];
                };
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                    object_key: components["parameters"]["ObjectKey"];
                };
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                /** @description Partial content */
                206: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                /** @description Not modified */
                304: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                400: components["responses"]["InvalidRequest"];
                404: components["responses"]["NotFound"];
                413: components["responses"]["PayloadTooLarge"];
                415: components["responses"]["UnsupportedMediaType"];
                /** @description Range not satisfiable */
                416: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content?: never;
                };
                422: components["responses"]["UnprocessableContent"];
            };
        };
        /** Update object metadata by path */
        patch: {
            parameters: {
                query?: never;
                header?: {
                    "If-Match"?: components["parameters"]["IfMatch"];
                };
                path: {
                    app_id: components["parameters"]["AppId"];
                    bucket: components["parameters"]["PublicBucketName"];
                    object_key: components["parameters"]["ObjectKey"];
                };
                cookie?: never;
            };
            requestBody: {
                content: {
                    "application/json": components["schemas"]["UpdateMedia"];
                };
            };
            responses: {
                /** @description Successful response */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": components["schemas"]["Media"];
                    };
                };
                400: components["responses"]["InvalidRequest"];
                401: components["responses"]["Unauthorized"];
                403: components["responses"]["Forbidden"];
                404: components["responses"]["NotFound"];
                409: components["responses"]["Conflict"];
            };
        };
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        AccessKey: {
            access_key_id: string;
            created_at: string;
            expires_at?: string | null;
            name: string;
            permissions: components["schemas"]["Permission"][];
            revoked_at?: string | null;
            secret_last_four: string;
        };
        AdminApplication: {
            app_id: string;
            created_at: string;
            id: string;
            name: string;
            owner_user_id: string;
            /** Format: int64 */
            quota_bytes: number;
            /** Format: int64 */
            reserved_bytes: number;
            updated_at: string;
            /** Format: int64 */
            used_bytes: number;
        };
        AdminAudit: {
            action: string;
            actor_id: string;
            /** @enum {string} */
            actor_type: "user" | "access_key" | "system";
            application_id: string;
            created_at: string;
            id: string;
            request_id: string;
            summary: {
                [key: string]: unknown;
            };
            target_id: string;
            target_type: string;
        };
        AdminJob: {
            action: string;
            application_id: string;
            /** Format: int64 */
            attempt_count: number;
            created_at: string;
            error_summary?: string | null;
            /** Format: int64 */
            failed_items: number;
            id: string;
            /** Format: int64 */
            max_attempts: number;
            /** @enum {string} */
            state: "pending" | "running" | "completed" | "failed" | "cancelled";
            /** Format: int64 */
            succeeded_items: number;
            /** Format: int64 */
            total_items: number;
            updated_at: string;
        };
        AdminSettings: {
            /**
             * Format: int64
             * @description Per-download bandwidth limit in bytes per second; null means unlimited.
             */
            download_bytes_per_second: number | null;
            /** Format: date-time */
            updated_at: string;
        };
        AdminStorage: {
            /** Format: int64 */
            disk_available_bytes: number;
            /** Format: int64 */
            disk_total_bytes: number;
            /** Format: int64 */
            media_objects: number;
            /** Format: int64 */
            quota_bytes: number;
            /** Format: int64 */
            reserved_bytes: number;
            /** Format: int64 */
            used_bytes: number;
            /** Format: int64 */
            variant_bytes: number;
            /** Format: int64 */
            variants: number;
        };
        AdminUpdateApplicationQuota: {
            /** Format: int64 */
            quota_bytes: number;
        };
        AdminUpdateSettings: {
            /**
             * Format: int64
             * @description Per-download bandwidth limit in bytes per second; null means unlimited.
             */
            download_bytes_per_second: number | null;
        };
        AdminUpdateUserStatus: {
            status: string;
        };
        AdminUser: {
            created_at: string;
            email: string;
            email_verified_at?: string | null;
            id: string;
            last_login_at?: string | null;
            /** @enum {string} */
            status: "pending_verification" | "active" | "suspended";
            /** @enum {string} */
            system_role: "user" | "admin";
            updated_at: string;
        };
        Application: {
            app_id: string;
            id: string;
            name: string;
            /** Format: int64 */
            quota_bytes: number;
            /** Format: int64 */
            reserved_bytes: number;
            /** Format: int64 */
            used_bytes: number;
        };
        AsyncJob: {
            action: components["schemas"]["AsyncJobAction"];
            application_id: string;
            /** Format: int64 */
            attempt_count: number;
            /** Format: date-time */
            cancelled_at: string | null;
            /** Format: date-time */
            completed_at: string | null;
            /** Format: date-time */
            created_at: string;
            error_summary: string | null;
            /** Format: date-time */
            failed_at: string | null;
            /** Format: int64 */
            failed_items: number;
            id: string;
            /** Format: int64 */
            max_attempts: number;
            /** Format: date-time */
            next_attempt_at: string | null;
            operation_scope: string;
            request_id: string | null;
            /** Format: date-time */
            started_at: string | null;
            state: components["schemas"]["AsyncJobState"];
            /** Format: int64 */
            succeeded_items: number;
            /** Format: int64 */
            total_items: number;
            /** Format: date-time */
            updated_at: string;
        };
        AsyncJobAction: {
            /** Format: int64 */
            ttl_seconds?: number | null;
            /** @enum {string} */
            type: "update_ttl_seconds";
        } | {
            /** @enum {string} */
            type: "update_visibility";
            visibility: components["schemas"]["Visibility"];
        } | {
            /** @enum {string} */
            type: "delete";
        };
        AsyncJobDetails: {
            item_results: components["schemas"]["AsyncJobItemResult"][];
            job: components["schemas"]["AsyncJob"];
        };
        AsyncJobItemResult: {
            application_id: string;
            /** Format: int32 */
            attempt_count: number;
            /** Format: date-time */
            completed_at: string | null;
            error_code: string | null;
            error_summary: string | null;
            job_id: string;
            media_id: string;
            /** Format: int32 */
            ordinal: number;
            result: unknown;
            /** Format: date-time */
            started_at: string | null;
            state: components["schemas"]["AsyncJobItemState"];
            /** Format: date-time */
            updated_at: string;
        };
        /** @enum {string} */
        AsyncJobItemState: "pending" | "succeeded" | "failed" | "cancelled";
        AsyncJobReceipt: {
            already_existed: boolean;
            job: components["schemas"]["AsyncJob"];
        };
        /** @enum {string} */
        AsyncJobState: "pending" | "running" | "completed" | "failed" | "cancelled";
        AuditEvent: {
            action: string;
            actor_id: string;
            actor_type: string;
            created_at: string;
            id: string;
            request_id: string;
            summary: unknown;
            target_id: string;
            target_type: string;
        };
        AuthStatus: {
            status: string;
        };
        BatchItemResult: {
            error?: null | {
                code: string;
                message: string;
            };
            media_id: string;
            result?: unknown;
            state: string;
        };
        BatchMediaRequest: {
            action: components["schemas"]["AsyncJobAction"];
            media_ids: string[];
        };
        BatchMediaResponse: {
            results: components["schemas"]["BatchItemResult"][];
        };
        Bucket: {
            allowed_mime_types: string[];
            /** Format: int64 */
            default_ttl_seconds?: number | null;
            id: string;
            lifecycle_rules: components["schemas"]["LifecycleRule"][];
            /** Format: int64 */
            max_object_size?: number | null;
            name: string;
            visibility: components["schemas"]["Visibility"];
        };
        Capabilities: {
            archive_restore: boolean;
            deployment_profile: string;
            image_processing: boolean;
            resumable_upload: boolean;
            s3_gateway: boolean;
            storage: string[];
            video_processing: boolean;
        };
        CompleteUploadSession: {
            sha256: string;
        };
        CompleteUploadSessionResponse: {
            already_completed: boolean;
            event_id: string;
            media: components["schemas"]["Media"];
            upload_id: string;
        };
        CreateAccessKey: {
            expires_at?: string | null;
            name: string;
            permissions: components["schemas"]["Permission"][];
        };
        CreateAccessKeyResponse: {
            access_key_id: string;
            app_id: string;
            expires_at?: string | null;
            secret_access_key: string;
        };
        CreateApplication: {
            name: string;
        };
        CreateBucket: {
            allowed_mime_types?: string[];
            /** Format: int64 */
            default_ttl_seconds?: number | null;
            lifecycle_rules?: components["schemas"]["LifecycleRule"][];
            /** Format: int64 */
            max_object_size?: number | null;
            name: string;
            visibility?: null | components["schemas"]["Visibility"];
        };
        CreateUploadSession: {
            bucket: string;
            content_type: string;
            display_name?: string | null;
            /** Format: int64 */
            expected_size: number;
            extension?: string | null;
            metadata?: unknown;
            object_key?: string | null;
            original_name?: string | null;
            /** Format: int64 */
            ttl_seconds?: number | null;
            visibility?: null | components["schemas"]["Visibility"];
        };
        CreateUploadSessionResponse: {
            bucket_id: string;
            expected_mime: string;
            /** Format: int64 */
            expected_size: number;
            expires_at: string;
            headers: {
                [key: string]: string;
            };
            media_id: string;
            method: string;
            object_key: string;
            upload_id: string;
            url: string;
        };
        CreateWebhook: {
            /** @default true */
            enabled: boolean;
            events: string[];
            url: string;
        };
        CreateWebhookResponse: {
            endpoint: components["schemas"]["Webhook"];
            secret: string;
        };
        Credentials: {
            email: string;
            password: string;
        };
        Error: {
            error: {
                code: string;
                message: string;
                request_id: string;
            };
        };
        ForgotPassword: {
            email: string;
        };
        ForgotPasswordResponse: {
            message: string;
            reset_token?: string;
        };
        LifecycleRule: {
            /** Format: int64 */
            duration_seconds: number;
            enabled: boolean;
            id: string;
            prefix: string;
            /** @enum {string} */
            type: "expire_after";
        } | {
            /** Format: int32 */
            count: number;
            enabled: boolean;
            id: string;
            prefix: string;
            /** @enum {string} */
            type: "keep_latest";
        };
        Me: {
            app_id: string;
            application_id: string;
            email: string;
            /** Format: int64 */
            quota_bytes: number;
            /** Format: int64 */
            reserved_bytes: number;
            /** @enum {string} */
            system_role: "user" | "admin";
            /** Format: int64 */
            used_bytes: number;
        };
        Media: {
            bucket_id: string;
            created_at: string;
            display_name: string;
            expires_at?: string | null;
            /** Format: int32 */
            height?: number | null;
            id: string;
            metadata: unknown;
            mime: string;
            object_key: string;
            /** Format: int64 */
            revision: number;
            sha256: string;
            /** Format: int64 */
            size_bytes: number;
            state: string;
            updated_at: string;
            visibility?: null | components["schemas"]["Visibility"];
            /** Format: int32 */
            width?: number | null;
        };
        MediaPage: {
            common_prefixes: string[];
            items: components["schemas"]["Media"][];
            next_cursor: string | null;
        };
        OneTimeToken: {
            token: string;
        };
        /** @enum {string} */
        Permission: "application:read" | "bucket:list" | "bucket:manage" | "media:list" | "media:read" | "media:upload" | "media:update" | "media:delete" | "webhook:manage";
        RegistrationResponse: {
            email: string;
            /** @enum {string} */
            status: "pending_verification";
            verification_token?: string;
        };
        ResendVerificationResponse: {
            message: string;
            verification_token?: string;
        };
        ResetPassword: {
            password: string;
            token: string;
        };
        Session: {
            created_at: string;
            created_ip?: string | null;
            expires_at: string;
            id: string;
            is_current: boolean;
            last_seen_at: string;
            last_seen_ip?: string | null;
            user_agent_summary?: string | null;
        };
        SignedMediaUrl: {
            expires_at: string;
            url: string;
        };
        UpdateAccessKey: {
            expires_at?: string | null;
            name?: string | null;
            permissions?: components["schemas"]["Permission"][] | null;
        };
        UpdateApplication: {
            name: string;
        };
        UpdateBucket: {
            allowed_mime_types?: string[] | null;
            /** Format: int64 */
            default_ttl_seconds?: number | null;
            lifecycle_rules?: components["schemas"]["LifecycleRule"][] | null;
            /** Format: int64 */
            max_object_size?: number | null;
            visibility?: null | components["schemas"]["Visibility"];
        };
        UpdateMedia: {
            display_name?: string | null;
            metadata?: unknown;
            /** Format: int64 */
            ttl_seconds?: number | null;
            visibility?: null | components["schemas"]["Visibility"];
        };
        UpdateWebhook: {
            enabled?: boolean | null;
            events?: string[] | null;
            /** @default false */
            rotate_secret: boolean;
            url?: string | null;
        };
        UpdateWebhookResponse: {
            endpoint: components["schemas"]["Webhook"];
            secret?: string | null;
        };
        UploadMedia: {
            bucket: string;
            display_name?: string | null;
            file: string;
            metadata?: unknown;
            object_key?: string | null;
            /** Format: int64 */
            ttl_seconds?: number | null;
            visibility?: null | components["schemas"]["Visibility"];
        };
        UploadSession: {
            bucket_id: string;
            cancelled_at: string | null;
            completed_at: string | null;
            created_at: string;
            expected_mime: string;
            /** Format: int64 */
            expected_size: number;
            expired_at: string | null;
            expires_at: string;
            media_id: string;
            object_key: string;
            /** @enum {string} */
            state: "pending" | "completed" | "cancelled" | "expired";
            updated_at: string;
            upload_id: string;
            upload_target?: null | components["schemas"]["UploadTarget"];
        };
        UploadTarget: {
            expires_at: string;
            headers: {
                [key: string]: string;
            };
            method: string;
            url: string;
        };
        /** @enum {string} */
        Visibility: "public" | "private";
        Webhook: {
            created_at: string;
            enabled: boolean;
            events: string[];
            id: string;
            updated_at: string;
            url: string;
        };
        WebhookDelivery: {
            /** Format: int32 */
            attempt_count: number;
            created_at: string;
            dead_lettered_at?: string | null;
            delivered_at?: string | null;
            endpoint_id: string;
            event_id: string;
            event_type: string;
            last_error?: string | null;
            last_replayed_at?: string | null;
            /** Format: int32 */
            last_response_status?: number | null;
            next_attempt_at?: string | null;
            /** Format: int32 */
            replay_count: number;
            /** @enum {string} */
            status: "pending" | "delivered" | "dead_lettered";
            updated_at: string;
        };
        WebhookDeliveryPage: {
            items: components["schemas"]["WebhookDelivery"][];
            next_cursor: string | null;
        };
    };
    responses: {
        /** @description State or idempotency conflict */
        Conflict: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Permission denied */
        Forbidden: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Invalid request */
        InvalidRequest: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Resource not found */
        NotFound: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Payload too large */
        PayloadTooLarge: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Rate limit exceeded */
        RateLimited: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Authentication required */
        Unauthorized: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Dependency unavailable */
        Unavailable: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Content or policy validation failed */
        UnprocessableContent: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
        /** @description Unsupported media type */
        UnsupportedMediaType: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["Error"];
            };
        };
    };
    parameters: {
        AccessKeyId: string;
        AdminLimit: number;
        AppId: string;
        ApplicationContext: string;
        ApplicationId: string;
        BatchIdempotencyKey: string;
        BucketName: string;
        ContentLength: number;
        ContentType: string;
        CreatedBefore: string;
        CreatedFrom: string;
        Cursor: string;
        Delimiter: string;
        DeliveryStatus: string;
        EventId: string;
        IdempotencyKey: string;
        IfMatch: string;
        IfNoneMatch: string;
        JobId: string;
        Limit: number;
        MediaBucket: string;
        MediaId: string;
        MediaMime: string;
        MediaStatus: string;
        ObjectKey: string;
        ObjectPrefix: string;
        PublicBucketName: string;
        Range: string;
        SessionId: string;
        UploadSessionId: string;
        UserId: string;
        VariantBackground: string;
        VariantBlur: number;
        VariantCrop: "center" | "top" | "bottom" | "left" | "right";
        VariantFit: "cover" | "contain" | "inside";
        VariantFormat: "jpeg" | "png" | "webp";
        VariantHeight: number;
        VariantQuality: number;
        VariantWidth: number;
        WebhookId: string;
    };
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export type operations = Record<string, never>;
