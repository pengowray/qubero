/**
 * DefaultResourceFactory.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.app.*;
import java.util.Collection;

public class DefaultResourceFactory extends ResourceFactory
{
    public Resource wrap(Object o) {
	if (o instanceof Resource)
	    return wrap((Resource)o);
        
        if (o instanceof OpenFile)
	    return wrap((OpenFile)o);
        
        return new ContainerResource(o, null);
    }

    // dont wrap (already wrapped) resources
    public Resource wrap(Resource o) {
	return o;
    }
    
    public Resource wrap(OpenFile of) {
	return new OpenFileResource(of.getResourceList(), of);
    }

}

