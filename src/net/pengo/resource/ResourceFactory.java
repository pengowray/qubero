/**
 * ResourceFactory.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

//xxx: should just be an interface.

public abstract class ResourceFactory {
    
    private static ResourceFactory defaultRF;
    public static ResourceFactory getDefault() {
	if (defaultRF == null)
		defaultRF = new DefaultResourceFactory();
	
	return defaultRF;
    }
    
   abstract public Resource wrap(Object o);
}

