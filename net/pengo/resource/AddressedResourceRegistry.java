/**
 * AddressedResourceRegistry.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;
import net.pengo.app.OpenFile;
import java.lang.reflect.InvocationTargetException;



public class AddressedResourceRegistry
{
    static private AddressedResourceRegistry singleton;
    
    /** the singleton constructor */
    static public AddressedResourceRegistry instance() {
	if (singleton == null)
	    singleton = new AddressedResourceRegistry();
	
	return singleton;
	
    }
    
    private AddressedResourceRegistry() {}
    
    /** The list of AddressedResouce classes */
    public Class[] list() {
	return new Class[] {
	    BooleanAddressedResource.class,
	    IntAddressedResource.class
	};
    }
    
    /**
     * instanciate an AddressedResource */
    public AddressedResource newAddressedResource(Class cl, OpenFile openFile, SelectionResource selRes) {
	if (!(cl.isInstance(AddressResource.class))) {
	    //fixme: error
	    return null;
	}
	
	try {
	    return (AddressedResource)cl.getConstructor(new Class[] {OpenFile.class, SelectionResource.class}).newInstance(new Object[] {openFile, selRes});
	} catch (SecurityException e) {} catch (InvocationTargetException e) {} catch (NoSuchMethodException e) {} catch (InstantiationException e) {} catch (IllegalAccessException e) {} catch (IllegalArgumentException e) {}
	return null;
    }
    
    
}

