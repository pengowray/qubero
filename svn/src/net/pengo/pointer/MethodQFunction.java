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

import java.lang.reflect.Method;

import net.pengo.resource.JavaType;
import net.pengo.resource.Resource;
import net.pengo.resource.SignatureTypeResource;

/**
 * @author Peter Halasz
 */
public class MethodQFunction extends QFunction {
    private Method method;
    
    /**
     * @param openFile
     */
    public MethodQFunction() {
        // TODO Auto-generated constructor stub
    }
    
    //wrap a method
    //eventually this should be replaced with some more formal way of converting types
    public void setValue(Method m) {
        this.method = m;
        //this.obj = obj;
        setName(m.getName()); //FIXME: umm? should i be doing this?
        //Class[] param = m.getParameterTypes();
        //Class ret = m.getReturnType();
    }

    public void invoke(SmartPointer result, Resource obj, Resource[] param){
        try {
            Object ret;
            ret = method.invoke(obj, param);
            result.setValue((Resource)ret);
        } catch (Exception e) {
            //FIXME
            e.printStackTrace();
        }
    }

    
    public SignatureTypeResource callSignature() {
        Class result = method.getDeclaringClass();
        
        Class[] param = method.getParameterTypes();
        JavaType[] jparam = new JavaType[param.length];
        for (int i=0; i<param.length; i++) jparam[i] = new JavaType(param[i]);
        
        return new SignatureTypeResource(new JavaType(result),null,jparam);
    }
    

    public Resource[] getSources() {
        return null;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
    
    
}
