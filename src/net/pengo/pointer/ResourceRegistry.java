/**
 * ResourceRegistry.java
 *
 * All resouces.. for when you want to select one.
 *
 * For now this is simple.  Later there should be more defined structures to select from.
 *
 * Used to be called AddressedResourceRegistry.. i guess because Primative types can be re-created easily or something? and all addressed resources would be created thru this class. anyway i dunno but i don't do it like that now.
 *
 * @author Peter Halasz
 */

package net.pengo.pointer;

import java.lang.ref.WeakReference;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;

import net.pengo.resource.Resource;

public class ResourceRegistry {
    static private ResourceRegistry singleton;
    
    /** the singleton constructor */
    static synchronized public ResourceRegistry instance() {
	if (singleton == null)
	    singleton = new ResourceRegistry();
	
	return singleton;
    }
    // private constructor
    private ResourceRegistry() {}
    
    //// END singleton junk
    
    

    private List reg = new ArrayList(); // of WeakRefernces to QNodeResource's
    
    
    public void add(Resource qnr) {
        //SmartPointer sp = new JavaPointer(dr.getOpenFile(), dr.getClass());
        
        //xxx: can't do this because class might not be initialized yet
        //sp.setName("Autowrapper for: " + dr); //fixme: besides, it should update
        
        //sp.setName("Autowrapper"); //fixme
        
        reg.add(new WeakReference(qnr));
    }
    
    public List getAll() {
	Iterator it = reg.iterator();
	List allOfType = new ArrayList();
	
	while (it.hasNext()) {
	    WeakReference wr = (WeakReference)it.next();
	    Resource sp = (Resource)wr.get();
	    if (sp!=null) {
		allOfType.add(sp);
	    }
	}
	
	return allOfType;
    }
    
    public List getAllOfType(Class cl) {
	Iterator it = reg.iterator();
	List allOfType = new ArrayList();
	
	while (it.hasNext()) {
	    WeakReference wr = (WeakReference)it.next();
	    Resource sp = (Resource)wr.get();
	    if (sp!=null) {
		if (sp.isAssignableTo(cl))
		    allOfType.add(sp);
	    }
	}
	
	return allOfType;
    }
    
    /** The list of Resource classes? */
    /*
     public Class[] list() {
     return new Class[] {
     BooleanAddressedResource.class,
     IntAddressedResource.class
     };
     }
     */
    
    
    /**
     * instanciate an AddressedResource */
    /*
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
     */
    
    
}

