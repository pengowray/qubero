/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * JavaSmartPointer.java
 *
 Simple implementation of SmartPointer.
 Uses Java's class hierarchy to decide if a value is of the right type.
 
 * @author Peter Halasz
 */

package net.pengo.pointer;

import java.util.List;

import net.pengo.propertyEditor.ResourceSelectorPage;
import net.pengo.resource.Resource;

public class JavaPointer extends SmartPointer {
    private String classname;
    private Class type;
    
    public JavaPointer(String classname) {
        this.classname = classname;
        
        try {
            type = Class.forName(classname);
        } catch (ClassNotFoundException e) {
            //fixme: need proper handling
            e.printStackTrace();
        }
    }
    
    public JavaPointer(Class cls) {
        this.classname = cls.getName(); //fixme: is this the qualified name? should be
        this.type = cls;
    }
    
    
    public String valueDesc() {
    	Resource r = evaluate();
    	String desc;
    	
    	if (r == null) {
    		desc = "[null]";
    	} else {   	
    		desc = r.valueDesc();
    	}
        
        if (value == null)
        	return "[null]";
        
    	if (value.getName()==null)
            if (value.isPointer())
                return "(->"+desc+")";
            else
                return "("+desc+")";
        else
            if (value.isPointer())
                return value.getName() + " (->" + desc + ")";
            else
                return value.getName() + " (" + desc + ")";
    }
    
    public Class getType() {
        return type;
    }
    
    public void setValue(Resource value) {
        
        //fixme: need proper handling.
        //note: this replaces compile time checking with runtime checking. fixme: fix with assertions?
        if (!isValidValue(value)) {
            System.out.println("error.. value is of wrong type. expected " + type + " but got " + value.getClass());
        }
        
        super.setValue(value);
    }
    
    public boolean isValidValue(Resource value) {
        return type.isInstance(value.evaluate());
    }
    
    public boolean isAssignableFrom(Class cl) {
        return (type.isAssignableFrom(cl));
    }
    
    //fixme: is name correct? is assignableTo really opposite to assignableFrom
    public boolean isAssignableTo(Class cl) {
        return (cl.isAssignableFrom(type));
    }
    
    
    /** @return list of PropertyPage's */ 
    public List getPrimaryPages() {
        List pp = super.getPrimaryPages();
        pp.add(new ResourceSelectorPage(this));
        return pp;
    }        
    
    
}

