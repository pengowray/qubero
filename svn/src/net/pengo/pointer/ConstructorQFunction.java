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

/*
 * Created on Jan 1, 2004
 *
 */
package net.pengo.pointer;

import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;

import net.pengo.resource.JavaType;
import net.pengo.resource.Resource;
import net.pengo.resource.SelectionResource;
import net.pengo.resource.SignatureTypeResource;

/**
 * @author Peter Halasz
 */
public class ConstructorQFunction extends QFunction {
    private Constructor cons;
    
    /**
     * @param openFile
     */
    public ConstructorQFunction() {
        super();
    }
    
    
    public void setValue(Constructor cons) {
        this.cons = cons;
    }
    
    
    /* obj always = null */
    public void invoke(SmartPointer result, Resource obj, Resource[] param) {
	System.out.println("cons:" + cons);
        try {
            result.setValue((Resource)cons.newInstance(param));
        } catch (InstantiationException e) {
            // TODO: handle exception
            e.printStackTrace();
        } catch (InvocationTargetException e){
            e.printStackTrace();
	    System.err.println("real cause:");
	    e.getCause().printStackTrace();
        } catch (IllegalAccessException e) {
            e.printStackTrace();
        }
    }


    public SignatureTypeResource callSignature() {
        Class result = cons.getDeclaringClass();
        
        Class[] param = cons.getParameterTypes();
        JavaType[] jparam = new JavaType[param.length];
        for (int i=0; i<param.length; i++) jparam[i] = new JavaType(param[i]);
        
        return new SignatureTypeResource(new JavaType(result),null,jparam);
    }

    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }


    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
}
