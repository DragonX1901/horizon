package dev.horizonlab.bridge;

import com.google.gson.*;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.jar.JarFile;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.geom.builders.LayerDefinition;

/**
 * Bakes the entity model layers mods define in Java (static methods returning a
 * {@code LayerDefinition}, as {@code createBodyLayer} does) into the part trees Horizon draws mobs
 * with: the same JSON as VanillaEntityModelExporter. Runs on NeoForge's Mojang-named 1.21.1 client
 * classes. Output: {@code {"<class>#<method>": part}}.
 *
 * <p>Usage: {@code ModModelExporter OUT.json MOD.jar...}
 */
public final class ModModelExporter {
    private static Field cubes;
    private static Field children;
    private static Field polygons;
    private static Field vertices;
    private static Field normal;
    private static Field pos;
    private static Field u;
    private static Field v;

    @SuppressWarnings("unchecked")
    private static JsonObject part(ModelPart part) throws Exception {
        var o = new JsonObject();
        var pose = new JsonArray();
        for (float f : new float[] {part.x, part.y, part.z, part.xRot, part.yRot, part.zRot, part.xScale, part.yScale, part.zScale})
            pose.add(f);
        o.add("pose", pose);
        var polys = new JsonArray();
        var normals = new JsonArray();
        for (Object cube : (List<Object>) cubes.get(part)) {
            for (Object polygon : (Object[]) polygons.get(cube)) {
                var p = new JsonArray();
                for (Object vertex : (Object[]) vertices.get(polygon)) {
                    var at = (org.joml.Vector3f) pos.get(vertex);
                    var vx = new JsonArray();
                    for (float f : new float[] {at.x(), at.y(), at.z(), u.getFloat(vertex), v.getFloat(vertex)})
                        vx.add(Float.isFinite(f) ? f : 0.0F);
                    p.add(vx);
                }
                polys.add(p);
                var n = (org.joml.Vector3f) normal.get(polygon);
                var nj = new JsonArray();
                nj.add(n.x());
                nj.add(n.y());
                nj.add(n.z());
                normals.add(nj);
            }
        }
        o.add("polygons", polys);
        o.add("normals", normals);
        var kids = new JsonObject();
        for (var entry : ((Map<String, ModelPart>) children.get(part)).entrySet())
            kids.add(entry.getKey(), part(entry.getValue()));
        o.add("children", kids);
        return o;
    }

    private static Field field(Class<?> c, String name) throws Exception {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    public static void main(String[] args) throws Exception {
        // No Bootstrap: layer definitions are plain mesh data, and NeoForge's
        // patched registries refuse to start outside its loader.
        cubes = field(ModelPart.class, "cubes");
        children = field(ModelPart.class, "children");
        Class<?> cube = Class.forName("net.minecraft.client.model.geom.ModelPart$Cube");
        Class<?> polygon = Class.forName("net.minecraft.client.model.geom.ModelPart$Polygon");
        Class<?> vertex = Class.forName("net.minecraft.client.model.geom.ModelPart$Vertex");
        polygons = field(cube, "polygons");
        vertices = field(polygon, "vertices");
        normal = field(polygon, "normal");
        pos = field(vertex, "pos");
        u = field(vertex, "u");
        v = field(vertex, "v");

        var jars = new ArrayList<Path>();
        for (int i = 1; i < args.length; i++) jars.add(Path.of(args[i]));
        var urls = new URL[jars.size()];
        for (int i = 0; i < urls.length; i++) urls[i] = jars.get(i).toUri().toURL();
        var loader = new URLClassLoader(urls, ModModelExporter.class.getClassLoader());
        var out = new JsonObject();
        var supers = new JsonObject();
        var renderers = new JsonObject();
        int failed = 0;
        for (Path jar : jars) {
            try (var file = new JarFile(jar.toFile())) {
                for (var entry : Collections.list(file.entries())) {
                    String name = entry.getName();
                    if (!name.endsWith(".class") || name.startsWith("META-INF") || name.contains("module-info")) continue;
                    String className = name.substring(0, name.length() - 6).replace('/', '.');
                    Class<?> c;
                    Method[] methods;
                    try {
                        c = Class.forName(className, false, loader);
                        methods = c.getDeclaredMethods();
                    } catch (Throwable error) {
                        continue;
                    }
                    // A model whose layer its parent class defines (Aether's CockatriceModel and
                    // BipedBirdModel.createBodyLayer): matched through this map.
                    try {
                        if (net.minecraft.client.model.Model.class.isAssignableFrom(c) && c.getSuperclass() != null) {
                            supers.addProperty(c.getName(), c.getSuperclass().getName());
                        }
                    } catch (Throwable error) {
                        // Left out.
                    }
                    // A renderer drawing a vanilla model (Aether's FlyingCowRenderer and CowModel):
                    // the model class its superclass's type arguments name.
                    try {
                        if (net.minecraft.client.renderer.entity.EntityRenderer.class.isAssignableFrom(c)
                                && c.getGenericSuperclass() instanceof java.lang.reflect.ParameterizedType t) {
                            for (var arg : t.getActualTypeArguments()) {
                                Class<?> raw = arg instanceof Class<?> k ? k
                                        : arg instanceof java.lang.reflect.ParameterizedType p
                                                && p.getRawType() instanceof Class<?> k2 ? k2 : null;
                                if (raw != null && net.minecraft.client.model.Model.class.isAssignableFrom(raw)) {
                                    renderers.addProperty(c.getName(), raw.getName());
                                }
                            }
                        }
                    } catch (Throwable error) {
                        // Left out.
                    }
                    for (Method m : methods) {
                        if (!Modifier.isStatic(m.getModifiers()) || m.getReturnType() != LayerDefinition.class) continue;
                        Object[] arguments = defaults(m.getParameterTypes());
                        if (arguments == null) continue;
                        try {
                            m.setAccessible(true);
                            var layer = (LayerDefinition) m.invoke(null, arguments);
                            out.add(c.getName() + "#" + m.getName(), part(layer.bakeRoot()));
                        } catch (Throwable error) {
                            if (failed++ < 3) {
                                Throwable cause = error.getCause() != null ? error.getCause() : error;
                                System.err.println("failed " + c.getName() + "#" + m.getName() + ": " + cause);
                            }
                        }
                    }
                }
            }
        }
        out.add("__supers__", supers);
        out.add("__renderers__", renderers);

        Files.writeString(Path.of(args[0]), new Gson().toJson(out), StandardCharsets.UTF_8);
        System.out.println(out.size() + " layers, " + failed + " failed");
    }

    /**
     * The arguments a layer method takes as its default look: no cube deformation (the
     * {@code createBodyLayer(CubeDeformation)} pattern), zeros and false; null when it takes
     * something else.
     */
    private static Object[] defaults(Class<?>[] types) {
        Object[] out = new Object[types.length];
        for (int i = 0; i < types.length; i++) {
            Class<?> t = types[i];
            if (t == net.minecraft.client.model.geom.builders.CubeDeformation.class) {
                out[i] = net.minecraft.client.model.geom.builders.CubeDeformation.NONE;
            } else if (t == float.class) {
                out[i] = 0f;
            } else if (t == double.class) {
                out[i] = 0d;
            } else if (t == int.class) {
                out[i] = 0;
            } else if (t == boolean.class) {
                out[i] = false;
            } else {
                return null;
            }
        }
        return out;
    }
}
