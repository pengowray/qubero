/*
 * Created on Jan 17, 2004
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.resource;

/**
 * @author Peter Halasz
 *
 *
i've been thinking about giving user space programs more awareness of 
"regions".. so a program can create its own region-specific variables.. 
basically global variables that are specific to anything allocated within 
a specific region.. so if you have a large number of objects that would 
otherwise have pointers to the one object you can just stick them all in 
a "region"..

*/
public class Region extends Resource {

    /**
     * 
     */
    public Region() {
        super();
        // TODO Auto-generated constructor stub
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#getSources()
     */
    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }

}
