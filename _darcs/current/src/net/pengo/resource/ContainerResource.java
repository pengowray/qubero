/**
 * ContainerResource.java
 *
 * Generic container resource. Put anything into a resource.
 * "Double click for details." Created for debugging.
 *
 * @author Peter Halasz
 */

package net.pengo.resource;

public class ContainerResource extends Resource
{
	Object o;
	
	public ContainerResource(Object o) {
		this.o = o;
	}
	
	public void doubleClickAction() {
		super.doubleClickAction();
	    //System.out.println("  " + o.getClass() + " -- " + o);
	}

	public String valueDesc() {
		if (o==null)
			return "[NULL]";
		
		return o.toString();
	}

//	public String getName() {
//	    if (super.getName() == null) {
//	        return o.toString();
//	    }
//	    
//	    return super.getName();
//	}
	
	public String getTypeName() {
		if (o==null)
			return "[NULL container]";
		
	    return shortTypeName(o.getClass());
	}
	
	
	public boolean isPointer() {
	    return true;
	}

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#getSources()
     */
    public Resource[] getSources() {
        return new Resource[]{};
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
}

