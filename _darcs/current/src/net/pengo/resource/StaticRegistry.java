/*
 * Created on Jan 17, 2004
 *
 * For storing names and strings and stuff
 */
package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;

/**
 * @author Peter Halasz
 */
public class StaticRegistry extends Resource {
    
    public final SmartPointer reg = new JavaPointer("net.pengo.resource.ListResource");
    
    public StaticRegistry() {
        super();
        // TODO Auto-generated constructor stub
    }

    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }
    
}
