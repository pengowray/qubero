/**
 * ContainerResource.java
 *
 * Generic container resource. Put anything into a resource.
 * "Double click for details." Created for debugging.
 *
 * @author Peter Halasz
 */

package net.pengo.resource;

import net.pengo.app.OpenFile;

public class ContainerResource extends Resource
{
	Object o;
	
	public ContainerResource(Object o, OpenFile openFile) {
		super(openFile);
		this.o = o;
	}
	
	public void doubleClickAction() {
		super.doubleClickAction();
        System.out.println("  " + o.getClass() + " -- " + o);
	}
	
	public String toString() {
		return "RES:" + o.toString();
	}
		
		
}

